use clap::Parser;

/// Launch Claude Code on the least-used of several accounts.
#[derive(Debug, Parser)]
#[command(name = "ccx", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
