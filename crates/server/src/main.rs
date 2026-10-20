// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use uuid::Uuid;

use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};

const SERVER_INPUT_FORMATS: &str = r#"Input formats:
  server <session-name>
  server issue-token <session> <read|read_write|admin> [--ttl <seconds>]
  server health <session>
  server workspace-serve --data-dir <DIR> [--activate-workspace] [--https-listen <ADDR>] [--mtls-listen <ADDR>]
  server workspace-init --data-dir <DIR> --operator <NAME> --workspace <NAME>
  server workspace-tls-init --data-dir <DIR> --name <DNS_OR_IP> [--name <DNS_OR_IP>...]
  server workspace-tls-info --data-dir <DIR>
  server workspace-tls-renew --data-dir <DIR> [--name <DNS_OR_IP>...]
  server admin-info --data-dir <DIR>
  server reset-instance-key --data-dir <DIR> --principal <UUID> [--ttl-seconds <SECONDS>]
  server reset-owner-key --data-dir <DIR> --workspace <UUID> [--ttl-seconds <SECONDS>]"#;

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
    WorkspaceInit(WorkspaceInitArgs),
    WorkspaceServe(WorkspaceServeArgs),
    AdminInfo(LocalDataArgs),
    WorkspaceTlsInit(WorkspaceTlsArgs),
    WorkspaceTlsInfo(LocalDataArgs),
    WorkspaceTlsRenew(WorkspaceTlsRenewArgs),
    ResetInstanceKey(ResetInstanceArgs),
    ResetOwnerKey(ResetOwnerArgs),
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

#[derive(Args)]
struct LocalDataArgs {
    #[arg(long, value_name = "DIR")]
    data_dir: PathBuf,
}

#[derive(Args)]
struct WorkspaceServeArgs {
    #[command(flatten)]
    data: LocalDataArgs,
    #[arg(
        long,
        help = "Persist workspace mode; native access will then be refused"
    )]
    activate_workspace: bool,
    #[arg(long, default_value = "127.0.0.1:8443")]
    https_listen: SocketAddr,
    #[arg(long, default_value = "127.0.0.1:8444")]
    mtls_listen: SocketAddr,
}

#[derive(Args)]
struct WorkspaceInitArgs {
    #[command(flatten)]
    data: LocalDataArgs,
    #[arg(long, value_name = "NAME")]
    operator: String,
    #[arg(long, value_name = "NAME")]
    workspace: String,
}

#[derive(Args)]
struct WorkspaceTlsArgs {
    #[command(flatten)]
    data: LocalDataArgs,
    #[arg(long = "name", value_name = "DNS_OR_IP", required = true)]
    names: Vec<String>,
}

#[derive(Args)]
struct WorkspaceTlsRenewArgs {
    #[command(flatten)]
    data: LocalDataArgs,
    #[arg(long = "name", value_name = "DNS_OR_IP")]
    names: Vec<String>,
}

#[derive(Args)]
struct ResetInstanceArgs {
    #[command(flatten)]
    data: LocalDataArgs,
    #[arg(long, value_name = "UUID")]
    principal: Uuid,
    #[arg(long, value_name = "SECONDS")]
    ttl_seconds: Option<u64>,
}

#[derive(Args)]
struct ResetOwnerArgs {
    #[command(flatten)]
    data: LocalDataArgs,
    #[arg(long, value_name = "UUID")]
    workspace: Uuid,
    #[arg(long, value_name = "SECONDS")]
    ttl_seconds: Option<u64>,
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
        return execute_command(command).await;
    }

    server::run_server(
        cli.session_name
            .as_deref()
            .context("clap should require a session name when no subcommand is present")?,
    )
    .await
}

async fn execute_command(command: Command) -> anyhow::Result<()> {
    match command {
        Command::WorkspaceServe(args) => serve_workspace(args).await,
        Command::IssueToken(args) => print_json(&server::init::issue_enrollment_token(
            &args.session,
            args.access_level.as_str(),
            args.ttl,
        )?),
        Command::Health(args) => print_json(&server::init::check_server_health(&args.session)?),
        Command::WorkspaceInit(args) => {
            let result = server::local_admin::bootstrap(
                &args.data.data_dir,
                &args.operator,
                &args.workspace,
            )?;
            print_json(&serde_json::json!({
                "instance_id": result.instance_id.to_string(),
                "workspace_id": result.workspace_id.to_string(),
                "principal_id": result.instance_key.principal_id.to_string(),
                "instance_admin_key": {"id": result.instance_key.id.to_string(), "secret": result.instance_key.secret()},
                "workspace_owner_key": {"id": result.owner_key.id.to_string(), "secret": result.owner_key.secret()}
            }))
        }
        Command::WorkspaceTlsInit(args) => print_json(&server::instance_tls::initialize(
            &args.data.data_dir,
            &args.names,
        )?),
        Command::WorkspaceTlsRenew(args) => {
            let names = if args.names.is_empty() {
                None
            } else {
                Some(args.names.as_slice())
            };
            print_json(&server::instance_tls::renew(&args.data.data_dir, names)?)
        }
        Command::WorkspaceTlsInfo(args) => print_json(&server::instance_tls::info(&args.data_dir)?),
        Command::AdminInfo(args) => print_json(&server::local_admin::info(&args.data_dir)?),
        Command::ResetInstanceKey(args) => print_reset(server::local_admin::reset_key(
            &args.data.data_dir,
            server::foundation::LocalKeyTarget::InstancePrincipal(args.principal),
            args.ttl_seconds,
        )?),
        Command::ResetOwnerKey(args) => print_reset(server::local_admin::reset_key(
            &args.data.data_dir,
            server::foundation::LocalKeyTarget::WorkspaceOwner(args.workspace),
            args.ttl_seconds,
        )?),
    }
}

async fn serve_workspace(args: WorkspaceServeArgs) -> anyhow::Result<()> {
    let server = server::workspace_http::WorkspaceServer::bind(server::workspace_http::Config {
        data_dir: args.data.data_dir,
        https_listen: args.https_listen,
        mtls_listen: args.mtls_listen,
        activate_workspace: args.activate_workspace,
    })
    .await?;
    print_json(&server.startup()?)?;
    let report = server
        .run_until(async {
            tokio::signal::ctrl_c()
                .await
                .context("cannot listen for shutdown")
        })
        .await?;
    print_json(&report)
}

fn print_reset(result: server::foundation::LocalKeyReset) -> anyhow::Result<()> {
    let workspace = match result.scope {
        server::foundation::AdminKeyScope::Instance => None,
        server::foundation::AdminKeyScope::Workspace(id) => Some(id.to_string()),
    };
    print_json(&serde_json::json!({
        "principal_id": result.key.principal_id.to_string(), "workspace_id": workspace,
        "key_id": result.key.id.to_string(), "secret": result.key.secret(),
        "expires_at": result.expires_at, "revoked_keys": result.revoked_keys
    }))
}

fn print_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    let serialized = serde_json::to_string_pretty(value)?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{serialized}")?;
    stdout.flush()?;
    Ok(())
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
    #[expect(
        clippy::let_underscore_must_use,
        reason = "printing help is best effort when the terminal has closed"
    )]
    let _ = error.print();
    if use_stderr {
        let mut stderr = io::stderr();
        #[expect(
            clippy::let_underscore_must_use,
            reason = "printing help is best effort when the terminal has closed"
        )]
        let _ = writeln!(stderr, "\n{input_formats}");
    } else {
        let mut stdout = io::stdout();
        #[expect(
            clippy::let_underscore_must_use,
            reason = "printing help is best effort when the terminal has closed"
        )]
        let _ = writeln!(stdout, "\n{input_formats}");
    }
    std::process::exit(exit_code);
}
