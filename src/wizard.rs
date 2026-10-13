//! Guided, multi-step prompts for the `ccx manage` commands (#21).
//!
//! A wizard asks for each value one step at a time. It shows the current
//! state and a summary before a change. A wizard starts only when the
//! command has no required arguments and stdin, stdout and stderr are
//! terminals. Scripts and pipes never see a wizard (ADR 0011).
//!
//! The wizards use multi-step `dialoguer` prompts through the [`Prompt`]
//! trait, so that tests can give the answers with a fake input source.
//! A wizard only collects values. The command changes files after the
//! summary and the last confirm, so a stop at a step changes nothing.

use std::io::IsTerminal;

use crate::config::Result;

/// Why a wizard stopped before its end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// The user pushed Esc (or Ctrl-C) at a step.
    Cancel,
    /// An error. Nothing changed.
    Fail(String),
}

impl From<String> for Stop {
    fn from(message: String) -> Self {
        Stop::Fail(message)
    }
}

/// The result of one wizard step.
pub type Step<T> = std::result::Result<T, Stop>;

/// A check of a text answer. An error is the reason to ask again.
pub type Check<'a> = &'a dyn Fn(&str) -> Result<()>;

/// The input source of a wizard.
pub trait Prompt {
    /// Shows a line of state or a summary.
    fn show(&mut self, line: &str);
    /// Asks for a line of text. An empty answer gives `default` when there
    /// is one. `check` must accept the answer, else the prompt asks again.
    fn text(&mut self, prompt: &str, default: Option<&str>, check: Check) -> Step<String>;
    /// Asks a yes/no question.
    fn confirm(&mut self, prompt: &str, default: bool) -> Step<bool>;
    /// Asks for one item of `items`. Returns its index.
    fn select(&mut self, prompt: &str, items: &[String], default: usize) -> Step<usize>;
}

/// True when a wizard can start: stdin, stdout and stderr are terminals.
/// `dialoguer` draws the prompts on stderr.
pub fn available() -> bool {
    std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal()
}

/// The error when a command needs a wizard but has no terminal. It names
/// the flags that the command needs.
pub fn no_terminal(missing: &str, usage: &str) -> String {
    format!("{missing}, and there is no terminal for the wizard. Nothing changed. Run `{usage}`")
}

/// Runs a wizard and turns a cancel into "Nothing changed.".
pub fn finish(result: Step<()>) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(Stop::Cancel) => {
            println!("Cancelled. Nothing changed.");
            Ok(())
        }
        Err(Stop::Fail(message)) => Err(message),
    }
}

/// The last question before a change. The flag path confirms with `-y` or
/// a yes/no prompt. The wizard path asks through its [`Prompt`].
pub enum Ask<'a> {
    Flags { yes: bool },
    Wizard(&'a mut dyn Prompt),
}

impl Ask<'_> {
    /// Asks `prompt`. With no terminal on the flag path, the error tells
    /// the user to run `retry`. A cancel in the wizard is a no.
    pub fn confirm(&mut self, prompt: &str, retry: &str) -> Result<bool> {
        match self {
            Ask::Flags { yes } => crate::adopt::confirm(*yes, prompt, retry),
            Ask::Wizard(p) => match p.confirm(prompt, false) {
                Ok(answer) => Ok(answer),
                Err(Stop::Cancel) => Ok(false),
                Err(Stop::Fail(message)) => Err(message),
            },
        }
    }

    /// True on the wizard path.
    pub fn is_wizard(&self) -> bool {
        matches!(self, Ask::Wizard(_))
    }
}

/// The [`Prompt`] on the terminal of the user, through `dialoguer`.
///
/// Esc cancels a yes/no or a select step. A text step of `dialoguer` does
/// not read Esc; Ctrl-C stops the process at each step. In both cases the
/// wizard has not changed a file.
pub struct Terminal;

fn stop(e: dialoguer::Error) -> Stop {
    let dialoguer::Error::IO(e) = e;
    if e.kind() == std::io::ErrorKind::Interrupted {
        Stop::Cancel
    } else {
        Stop::Fail(format!("cannot read the answer: {e}"))
    }
}

impl Prompt for Terminal {
    fn show(&mut self, line: &str) {
        println!("{line}");
    }

    fn text(&mut self, prompt: &str, default: Option<&str>, check: Check) -> Step<String> {
        let mut input = dialoguer::Input::<String>::new()
            .with_prompt(prompt)
            .allow_empty(default.is_none() && check("").is_ok())
            .validate_with(|answer: &String| check(answer));
        if let Some(default) = default {
            input = input.default(default.to_string());
        }
        input.interact_text().map_err(stop)
    }

    fn confirm(&mut self, prompt: &str, default: bool) -> Step<bool> {
        dialoguer::Confirm::new()
            .with_prompt(prompt)
            .default(default)
            .interact_opt()
            .map_err(stop)?
            .ok_or(Stop::Cancel)
    }

    fn select(&mut self, prompt: &str, items: &[String], default: usize) -> Step<usize> {
        dialoguer::Select::new()
            .with_prompt(prompt)
            .items(items)
            .default(default)
            .interact_opt()
            .map_err(stop)?
            .ok_or(Stop::Cancel)
    }
}

/// A fake input source for the tests: it gives the answers in order.
#[cfg(test)]
pub mod fake {
    use super::*;
    use std::collections::VecDeque;

    /// One answer.
    #[derive(Debug, Clone)]
    pub enum Reply {
        /// A text answer. An empty text takes the default.
        Text(&'static str),
        Yes,
        No,
        /// The index of a select item.
        Pick(usize),
        /// Esc (or Ctrl-C).
        Esc,
    }

    /// Gives the replies in order and keeps what the wizard showed.
    #[derive(Debug, Default)]
    pub struct Script {
        replies: VecDeque<Reply>,
        /// The lines of `show`, the prompts and the check errors.
        pub log: Vec<String>,
    }

    impl Script {
        pub fn new(replies: impl IntoIterator<Item = Reply>) -> Self {
            Script {
                replies: replies.into_iter().collect(),
                log: Vec::new(),
            }
        }

        /// True when the wizard used all the replies.
        pub fn done(&self) -> bool {
            self.replies.is_empty()
        }

        pub fn text_log(&self) -> String {
            self.log.join("\n")
        }

        fn next(&mut self, prompt: &str) -> Reply {
            self.log.push(format!("? {prompt}"));
            self.replies
                .pop_front()
                .unwrap_or_else(|| panic!("no reply for `{prompt}`; log:\n{}", self.text_log()))
        }
    }

    impl Prompt for Script {
        fn show(&mut self, line: &str) {
            self.log.push(line.to_string());
        }

        fn text(&mut self, prompt: &str, default: Option<&str>, check: Check) -> Step<String> {
            loop {
                let answer = match self.next(prompt) {
                    Reply::Text("") => default.unwrap_or("").to_string(),
                    Reply::Text(text) => text.to_string(),
                    Reply::Esc => return Err(Stop::Cancel),
                    other => panic!("`{prompt}` wants text, not {other:?}"),
                };
                match check(&answer) {
                    Ok(()) => return Ok(answer),
                    // The real prompt shows the error and asks again.
                    Err(e) => self.log.push(format!("error: {e}")),
                }
            }
        }

        fn confirm(&mut self, prompt: &str, _default: bool) -> Step<bool> {
            match self.next(prompt) {
                Reply::Yes => Ok(true),
                Reply::No => Ok(false),
                Reply::Esc => Err(Stop::Cancel),
                other => panic!("`{prompt}` wants yes or no, not {other:?}"),
            }
        }

        fn select(&mut self, prompt: &str, items: &[String], _default: usize) -> Step<usize> {
            match self.next(prompt) {
                Reply::Pick(i) if i < items.len() => Ok(i),
                Reply::Esc => Err(Stop::Cancel),
                other => panic!("`{prompt}` wants one of {items:?}, not {other:?}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{Reply, Script};
    use super::*;

    #[test]
    fn script_asks_again_after_a_failed_check() {
        let mut p = Script::new([Reply::Text("bad"), Reply::Text("")]);
        let check = |s: &str| {
            if s == "bad" {
                Err("not good".to_string())
            } else {
                Ok(())
            }
        };
        assert_eq!(p.text("Name", Some("good"), &check), Ok("good".into()));
        assert!(p.text_log().contains("error: not good"));
        assert!(p.done());
    }

    #[test]
    fn a_cancel_in_the_wizard_is_a_no_at_the_last_question() {
        let mut p = Script::new([Reply::Esc, Reply::Yes]);
        let mut ask = Ask::Wizard(&mut p);
        assert!(ask.is_wizard());
        assert_eq!(ask.confirm("Continue?", ""), Ok(false));
        assert_eq!(ask.confirm("Continue?", ""), Ok(true));
        assert_eq!(Ask::Flags { yes: true }.confirm("Continue?", ""), Ok(true));
    }

    #[test]
    fn finish_turns_a_cancel_into_success() {
        assert_eq!(finish(Err(Stop::Cancel)), Ok(()));
        assert_eq!(finish(Err(Stop::Fail("x".into()))), Err("x".into()));
    }

    #[test]
    fn no_terminal_names_the_flags() {
        let e = no_terminal("missing the account name", "ccx manage rm NAME -y");
        assert!(e.contains("missing the account name"), "{e}");
        assert!(e.contains("`ccx manage rm NAME -y`"), "{e}");
    }
}
