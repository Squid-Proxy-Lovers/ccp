// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::io::{self, Write};

use clap::{Args, Parser, Subcommand};

const SERVER_INPUT_FORMATS: &str = r#"Input formats:
  server [initial-session]
  server create-session <session>"#;

#[derive(Parser)]
#[command(name = "server", args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(value_name = "session-name")]
    session_name: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    CreateSession(CreateSessionArgs),
}

#[derive(Args)]
struct CreateSessionArgs {
    #[arg(value_name = "session")]
    session: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = parse_cli();

    if let Some(command) = cli.command {
        match command {
            Command::CreateSession(args) => {
                server::init::initialize_plain_server(None)?;
                let session_id = server::init::create_session(&args.session)?;
                println!("session={} session_id={}", args.session, session_id);
                return Ok(());
            }
        }
    }

    server::run_plain_server(cli.session_name.as_deref()).await
}

fn parse_cli() -> Cli {
    // simple helper to parse the cli arguments
    match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => exit_with_cli_error(error, SERVER_INPUT_FORMATS),
    }
}

fn exit_with_cli_error(error: clap::Error, input_formats: &str) -> ! {
    // exit with an error code and print the input formats
    // Input formats:
    //     server <session-name>
    //     server issue-token <session> <read|read_write> [--ttl <seconds>]
    let use_stderr = error.use_stderr();
    let exit_code = error.exit_code();
    let _ = error.print();
    if use_stderr {
        let mut stderr = io::stderr();
        let _ = writeln!(stderr, "\n{input_formats}");
    } else {
        let mut stdout = io::stdout();
        let _ = writeln!(stdout, "\n{input_formats}");
    }
    std::process::exit(exit_code);
}
