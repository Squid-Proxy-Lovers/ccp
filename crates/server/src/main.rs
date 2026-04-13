// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::io::{self, Write};

use clap::{Args, Parser, Subcommand, ValueEnum};

const SERVER_INPUT_FORMATS: &str = r#"Input formats:
  server <session-name>
  server issue-token <session> <read|read_write|admin> [--ttl <seconds>]
  server health <session>"#;

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
    IssueToken(IssueTokenArgs),
    Health(HealthArgs),
}

#[derive(Args)]
struct HealthArgs {
    #[arg(value_name = "session")]
    session: String,
}

#[derive(Args)]
struct IssueTokenArgs {
    #[arg(value_name = "session")]
    session: String,
    #[arg(value_name = "read|read_write|admin")]
    access_level: AccessLevel,
    #[arg(long, value_name = "seconds")]
    ttl: Option<u64>,
}

#[derive(Clone, ValueEnum)]
enum AccessLevel {
    Read,
    #[value(name = "read_write")]
    ReadWrite,
    Admin,
}

impl AccessLevel {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::ReadWrite => "read_write",
            Self::Admin => "admin",
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = parse_cli();

    if let Some(command) = cli.command {
        match command {
            Command::IssueToken(args) => {
                let token = server::init::issue_enrollment_token(
                    &args.session,
                    args.access_level.as_str(),
                    args.ttl,
                )?;
                println!("{}", serde_json::to_string_pretty(&token)?);
                return Ok(());
            }
            Command::Health(args) => {
                let health = server::init::check_server_health(&args.session)?;
                println!("{}", serde_json::to_string_pretty(&health)?);
                return Ok(());
            }
        }
    }

    server::run_server(
        cli.session_name
            .as_deref()
            .expect("clap should require a session name when no subcommand is present"),
    )
    .await
}

fn parse_cli() -> Cli {
    // simple helper to parse the cli arguments
    match Cli::try_parse() {
        Ok(cli) => {
            if cli.command.is_none() && cli.session_name.is_none() {
                eprintln!("{SERVER_INPUT_FORMATS}");
                std::process::exit(2);
            }
            cli
        }
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
