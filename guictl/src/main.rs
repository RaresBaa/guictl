mod cli;
mod commands;
mod guide;
mod output;
mod platform;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    let pretty = cli.pretty;
    // Prose for an agent to read, so it's printed as is rather than as JSON.
    if matches!(cli.command, cli::Command::Guide) {
        print!("{}", guide::text());
        return;
    }
    match commands::dispatch(cli.command) {
        Ok(value) => {
            output::print_ok(value, pretty);
        }
        Err(err) => {
            output::print_err(&err, pretty);
            std::process::exit(err.exit_code);
        }
    }
}
