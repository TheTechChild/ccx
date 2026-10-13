//! End-to-end tests for `ccx manage shell-init zsh|bash|fish`.
//!
//! The golden files are in `testdata/shell-init/`. To update them, run
//! `CCX_UPDATE_GOLDEN=1 mise run test` and check the diff.
//!
//! The real-shell tests run each shell that is on PATH. Set
//! `CCX_TEST_SHELLS=zsh,bash,fish` to make a missing shell a failure (CI
//! does this).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CCX: &str = env!("CARGO_BIN_EXE_ccx");

/// A fixed synthetic config for the golden files.
const CONFIG: &str = r#"
[accounts.main]
dir = "~/.claude"

[accounts.angel2]
dir = "~/.claude-angel2"
aliases = ["claude2"]

[accounts.lab]
dir = "~/.claude-lab"
aliases = ["cl_lab", "c3"]
"#;

const SHELLS: [&str; 3] = ["zsh", "bash", "fish"];

struct Fixture {
    home: tempfile::TempDir,
}

impl Fixture {
    fn new(config: &str) -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        let dir = home.path().join(".config/ccx");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), config).unwrap();
        Fixture { home }
    }

    fn without_config() -> Self {
        Fixture {
            home: tempfile::tempdir().expect("temp HOME"),
        }
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    /// PATH: the dir of the ccx binary, the fake claude, then system dirs.
    fn path() -> String {
        let ccx_dir = Path::new(CCX).parent().unwrap();
        let fakebin = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fakebin");
        format!(
            "{}:{}:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin",
            ccx_dir.display(),
            fakebin.display()
        )
    }

    fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.env_clear()
            .env("HOME", self.home())
            .env("PATH", Self::path())
            .env("TERM", "dumb")
            .current_dir(self.home());
        cmd
    }

    fn shell_init(&self, shell: &str) -> Output {
        self.command(CCX)
            .args(["manage", "shell-init", shell])
            .output()
            .expect("run ccx")
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).unwrap()
}

/// Finds `shell` on PATH. A missing shell is a failure only when
/// `CCX_TEST_SHELLS` names it.
fn find_shell(shell: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found = std::env::split_paths(&path)
        .map(|dir| dir.join(shell))
        .find(|p| p.is_file());
    if found.is_none() {
        let required = std::env::var("CCX_TEST_SHELLS").unwrap_or_default();
        assert!(
            !required.split(',').any(|s| s.trim() == shell),
            "CCX_TEST_SHELLS needs {shell}, but it is not on PATH"
        );
        eprintln!("skip: {shell} is not on PATH");
    }
    found
}

/// Runs `script` in `shell` with no profile, and returns stdout. The script
/// fails the test when the shell exits with an error.
fn run_shell(fixture: &Fixture, shell: &Path, script: &str) -> String {
    let mut cmd = fixture.command(shell.to_str().unwrap());
    match shell.file_name().unwrap().to_str().unwrap() {
        "bash" => cmd.args(["--norc", "--noprofile", "-c", script]),
        "zsh" => cmd.args(["-f", "-c", script]),
        "fish" => cmd.args(["--no-config", "-c", script]),
        other => panic!("unknown shell {other}"),
    };
    let out = cmd.output().expect("run shell");
    assert!(
        out.status.success(),
        "{} failed\nstdout:\n{}\nstderr:\n{}",
        shell.display(),
        stdout(&out),
        stderr(&out)
    );
    stdout(&out)
}

// Output.

#[test]
fn golden_output_for_each_shell() {
    let fixture = Fixture::new(CONFIG);
    let update = std::env::var_os("CCX_UPDATE_GOLDEN").is_some();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/shell-init");
    for shell in SHELLS {
        let out = fixture.shell_init(shell);
        assert!(out.status.success(), "{shell}: {}", stderr(&out));
        let text = stdout(&out);
        let golden = dir.join(format!("ccx.{shell}"));
        if update {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&golden, &text).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&golden)
            .unwrap_or_else(|e| panic!("read {}: {e}", golden.display()));
        assert_eq!(
            text,
            want,
            "{shell}: output differs from {}; run with CCX_UPDATE_GOLDEN=1 to update",
            golden.display()
        );
    }
}

#[test]
fn each_alias_gives_one_line() {
    let fixture = Fixture::new(CONFIG);
    let want = [
        ("zsh", "alias claude2='ccx @angel2'"),
        ("zsh", "alias cl_lab='ccx @lab'"),
        ("zsh", "alias c3='ccx @lab'"),
        ("bash", "alias claude2='ccx @angel2'"),
        ("fish", "alias claude2 'ccx @angel2'"),
        ("fish", "alias c3 'ccx @lab'"),
    ];
    for (shell, line) in want {
        let text = stdout(&fixture.shell_init(shell));
        assert_eq!(
            text.lines().filter(|l| *l == line).count(),
            1,
            "{shell}: {line}"
        );
        assert_eq!(
            text.lines().filter(|l| l.starts_with("alias ")).count(),
            3,
            "{shell}"
        );
    }
}

#[test]
fn output_never_defines_claude() {
    let fixture = Fixture::new(CONFIG);
    for shell in SHELLS {
        let text = stdout(&fixture.shell_init(shell));
        for bad in [
            "alias claude=",
            "alias claude ",
            "function claude",
            "claude()",
        ] {
            assert!(!text.contains(bad), "{shell} output contains {bad:?}");
        }
    }
}

#[test]
fn claude_as_an_alias_is_an_error() {
    let fixture = Fixture::new("[accounts.main]\ndir = \"~/.claude\"\naliases = [\"claude\"]\n");
    let out = fixture.shell_init("bash");
    assert!(!out.status.success());
    assert!(stdout(&out).is_empty());
    assert!(
        stderr(&out).contains("never makes an alias `claude`"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn unsafe_alias_names_are_errors() {
    for alias in ["x; rm -rf ~", "it's", "a b", "2fast", "$(id)", "c-2"] {
        let config = format!(
            "[accounts.main]\ndir = \"~/.claude\"\naliases = [{}]\n",
            toml_string(alias)
        );
        let out = Fixture::new(&config).shell_init("zsh");
        assert!(!out.status.success(), "{alias}");
        assert!(stdout(&out).is_empty(), "{alias}");
        assert!(
            stderr(&out).contains("not a valid shell identifier"),
            "{alias}: {}",
            stderr(&out)
        );
    }
}

fn toml_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[test]
fn unknown_shell_is_an_error() {
    let out = Fixture::new(CONFIG).shell_init("tcsh");
    assert_eq!(out.status.code(), Some(2));
    assert!(stdout(&out).is_empty());
    let err = stderr(&out);
    assert!(err.contains("invalid value 'tcsh'"), "{err}");
    assert!(err.contains("zsh, bash, fish"), "{err}");
}

#[test]
fn no_config_gives_completions_only() {
    let fixture = Fixture::without_config();
    for shell in SHELLS {
        let out = fixture.shell_init(shell);
        assert!(out.status.success(), "{shell}: {}", stderr(&out));
        let text = stdout(&out);
        assert!(text.contains("manage"), "{shell}");
        assert!(!text.contains("alias "), "{shell}");
        assert!(!text.contains("@main"), "{shell}");
    }
}

#[test]
fn invalid_config_is_an_error() {
    let out = Fixture::new("[accounts.main]\n").shell_init("bash");
    assert!(!out.status.success());
    assert!(stdout(&out).is_empty());
    assert!(
        stderr(&out).contains("missing key `dir`"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn completions_name_all_commands_and_accounts() {
    let fixture = Fixture::new(CONFIG);
    for shell in SHELLS {
        let text = stdout(&fixture.shell_init(shell));
        for word in [
            "manage",
            "status",
            "usage",
            "handoff",
            "adopt",
            "list",
            "shell-init",
            "@main",
            "@angel2",
            "@lab",
        ] {
            assert!(text.contains(word), "{shell} completions lack {word}");
        }
    }
}

// Real shells.

/// The dry-run report of the command that `claude2 --x` runs.
const RUN_CLAUDE2: &str = "CCX_DRY_RUN=1 claude2 --x";

#[test]
fn bash_alias_runs_ccx_at_angel2() {
    let Some(bash) = find_shell("bash") else {
        return;
    };
    let fixture = Fixture::new(CONFIG);
    let script = format!(
        "shopt -s expand_aliases\n\
         eval \"$(ccx manage shell-init bash)\"\n\
         alias claude2\n\
         type -t claude\n\
         {RUN_CLAUDE2}\n"
    );
    let out = run_shell(&fixture, &bash, &script);
    let mut lines = out.lines();
    assert_eq!(lines.next(), Some("alias claude2='ccx @angel2'"));
    // `claude` is the program on PATH, not an alias or a function.
    assert_eq!(lines.next(), Some("file"));
    assert!(out.contains("account: angel2\n"), "{out}");
    assert!(out.contains("argv: [\"--x\"]\n"), "{out}");
}

#[test]
fn bash_completes_commands_and_accounts() {
    let Some(bash) = find_shell("bash") else {
        return;
    };
    let fixture = Fixture::new(CONFIG);
    let complete = |words: &str, cword: usize| {
        let script = format!(
            "eval \"$(ccx manage shell-init bash)\"\n\
             COMP_WORDS=({words} ''); COMP_CWORD={cword}\n\
             COMP_LINE='{words} '; COMP_POINT=${{#COMP_LINE}}\n\
             _ccx ccx '' ccx\n\
             printf '%s\\n' \"${{COMPREPLY[@]}}\"\n"
        );
        run_shell(&fixture, &bash, &script)
    };
    let top = complete("ccx", 1);
    for word in [
        "manage", "status", "usage", "handoff", "@main", "@angel2", "@lab",
    ] {
        assert!(top.lines().any(|l| l == word), "bash lacks {word}:\n{top}");
    }
    let manage = complete("ccx manage", 2);
    for word in ["adopt", "list", "shell-init"] {
        assert!(
            manage.lines().any(|l| l == word),
            "bash lacks {word}:\n{manage}"
        );
    }
}

#[test]
fn zsh_alias_runs_ccx_at_angel2() {
    let Some(zsh) = find_shell("zsh") else { return };
    let fixture = Fixture::new(CONFIG);
    // Without compinit, the output must not fail. `zsh -c` parses the
    // whole script before it runs, so the alias is used through `eval`.
    let script = format!(
        "eval \"$(ccx manage shell-init zsh)\"\n\
         alias claude2\n\
         whence -w claude\n\
         eval '{RUN_CLAUDE2}'\n"
    );
    let out = run_shell(&fixture, &zsh, &script);
    let mut lines = out.lines();
    assert_eq!(lines.next(), Some("claude2='ccx @angel2'"));
    assert_eq!(lines.next(), Some("claude: command"));
    assert!(out.contains("account: angel2\n"), "{out}");
    assert!(out.contains("argv: [\"--x\"]\n"), "{out}");
}

#[test]
fn zsh_registers_completions_after_compinit() {
    let Some(zsh) = find_shell("zsh") else { return };
    let fixture = Fixture::new(CONFIG);
    let script = "autoload -Uz compinit && compinit -u -d \"$HOME/.zcompdump\"\n\
                  eval \"$(ccx manage shell-init zsh)\"\n\
                  print -r -- \"$_comps[ccx]\"\n";
    let out = run_shell(&fixture, &zsh, script);
    assert_eq!(out, "_ccx\n");
}

#[test]
fn fish_alias_runs_ccx_at_angel2() {
    let Some(fish) = find_shell("fish") else {
        return;
    };
    let fixture = Fixture::new(CONFIG);
    // Fish aliases are functions. Run claude2 in the same fish process.
    let script = "ccx manage shell-init fish | source\n\
                  functions -q claude && echo claude-is-a-function\n\
                  type -t claude\n\
                  set -gx CCX_DRY_RUN 1\n\
                  claude2 --x\n\
                  set -e CCX_DRY_RUN\n\
                  complete -C 'ccx '\n\
                  complete -C 'ccx manage '\n";
    let out = run_shell(&fixture, &fish, script);
    assert!(!out.contains("claude-is-a-function"), "{out}");
    assert!(out.starts_with("file\n"), "{out}");
    assert!(out.contains("account: angel2\n"), "{out}");
    assert!(out.contains("argv: [\"--x\"]\n"), "{out}");
    for word in ["manage", "@angel2", "@lab", "adopt", "shell-init"] {
        assert!(
            out.lines().any(|l| l.split('\t').next() == Some(word)),
            "fish lacks {word}:\n{out}"
        );
    }
}

// The static completion files in `completions/`, which the release archives
// include. They have no `@name` tokens, because the archive has no config.

fn completion_file(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("completions")
        .join(name)
        .display()
        .to_string()
}

#[test]
fn archive_bash_completion_file_completes_commands() {
    let Some(bash) = find_shell("bash") else {
        return;
    };
    let fixture = Fixture::without_config();
    let script = format!(
        "source '{}'\n\
         COMP_WORDS=(ccx ''); COMP_CWORD=1\n\
         COMP_LINE='ccx '; COMP_POINT=${{#COMP_LINE}}\n\
         _ccx ccx '' ccx\n\
         printf '%s\\n' \"${{COMPREPLY[@]}}\"\n",
        completion_file("ccx.bash")
    );
    let out = run_shell(&fixture, &bash, &script);
    for word in ["manage", "status", "usage", "handoff"] {
        assert!(out.lines().any(|l| l == word), "bash lacks {word}:\n{out}");
    }
}

#[test]
fn archive_zsh_completion_file_loads_from_fpath() {
    let Some(zsh) = find_shell("zsh") else { return };
    let fixture = Fixture::without_config();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("completions");
    let script = format!(
        "fpath=('{}' $fpath)\n\
         autoload -Uz compinit && compinit -u -d \"$HOME/.zcompdump\"\n\
         print -r -- \"$_comps[ccx]\"\n",
        dir.display()
    );
    let out = run_shell(&fixture, &zsh, &script);
    assert_eq!(out, "_ccx\n");
}

#[test]
fn archive_fish_completion_file_completes_commands() {
    let Some(fish) = find_shell("fish") else {
        return;
    };
    let fixture = Fixture::without_config();
    let script = format!(
        "source '{}'\n\
         complete -C 'ccx '\n\
         complete -C 'ccx manage '\n",
        completion_file("ccx.fish")
    );
    let out = run_shell(&fixture, &fish, &script);
    for word in ["manage", "status", "adopt", "shell-init"] {
        assert!(
            out.lines().any(|l| l.split('\t').next() == Some(word)),
            "fish lacks {word}:\n{out}"
        );
    }
}
