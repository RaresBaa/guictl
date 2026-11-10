mod cli;
mod commands;
mod guide;
mod mcp;
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
    #[cfg(all(windows, not(guictl_stub)))]
    {
        use platform::os::elevate;
        let elevated = if let Some(base) = &cli.elevated_output {
            elevate::attach(base, cli.elevated_cwd.as_deref()).map(|()| None)
        } else if cli.elevate {
            elevate::relaunch(&cli.command).map(Some)
        } else {
            Ok(None)
        };
        match elevated {
            Ok(None) => {}
            Ok(Some(code)) => std::process::exit(code),
            Err(err) => {
                output::print_err(&err, pretty);
                std::process::exit(err.exit_code);
            }
        }
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
