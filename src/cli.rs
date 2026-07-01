use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

use crate::audits::docs::{self, DocsAuditOptions};
use crate::runner::Backend;

#[derive(Debug, Parser)]
#[command(
    name = "sdkcheck",
    version,
    about = "Audit whether an agent can actually follow your product docs and complete the intended flow.",
    long_about = "Audit whether an agent can actually follow your product docs and complete the intended flow.\n\nsdkcheck treats docs as executable infrastructure: it runs real commands in an isolated environment and writes an evidence report when the flow breaks.",
    after_help = "Run `sdkcheck run --help` to see audit options and examples."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[command(about = "Run an audit and write an evidence report.")]
    Run(RunCommand),
}

#[derive(Debug, Args)]
#[command(
    after_help = "Examples:\n  sdkcheck run --docs https://docs.example.com/api/latest/ --goal \"Install the SDK and make one successful example API request.\" --env EXAMPLE_API_KEY --env EXAMPLE_APP_KEY --env EXAMPLE_SITE\n  sdkcheck run --docs README.md --docs docs/quickstart.md --goal \"Install the SDK and complete the quickstart.\" --env ACME_API_KEY\n  sdkcheck run --docs docs/quickstart.md --workspace . --goal \"Install from this checkout and run the documented example.\" --backend local\n"
)]
struct RunCommand {
    #[arg(
        long = "docs",
        required = true,
        value_name = "PATH_OR_URL",
        help = "Documentation path or URL to seed the agent with. Pass multiple times."
    )]
    docs: Vec<String>,

    #[arg(
        long,
        value_name = "DIR",
        help = "Workspace directory to copy into the isolated audit runtime. Defaults to the current directory when local docs are used; URL-only audits start from an empty workspace."
    )]
    workspace: Option<PathBuf>,

    #[arg(
        long,
        help = "Plain-language goal the agent must complete by following the docs."
    )]
    goal: String,

    #[arg(
        long = "success",
        help = "Explicit success criterion for the audit. Pass multiple times."
    )]
    success_criteria: Vec<String>,

    #[arg(long, value_enum, default_value_t = Backend::Docker, help = "Where to execute the audit. Docker is the default and recommended backend.")]
    backend: Backend,

    #[arg(
        long,
        default_value = ".sdkcheck-work",
        help = "Directory for audit worktrees, virtual environments, and temporary files."
    )]
    workdir: PathBuf,

    #[arg(
        long,
        default_value = "reports/run.md",
        help = "Markdown evidence report path."
    )]
    output: PathBuf,

    #[arg(long, help = "Optional JSON evidence report path.")]
    json_output: Option<PathBuf>,

    #[arg(long, default_value_t = 900, help = "Per-command timeout in seconds.")]
    timeout_seconds: u64,

    #[arg(
        long = "env",
        help = "Environment variable name to forward into the audited runtime. Pass multiple times."
    )]
    forwarded_envs: Vec<String>,

    #[arg(
        long,
        help = "OpenAI-compatible base URL for the audit agent. Defaults to SDKCHECK_AGENT_BASE_URL or https://api.openai.com/v1."
    )]
    agent_base_url: Option<String>,

    #[arg(
        long,
        help = "Model name for the audit agent. Can also come from SDKCHECK_AGENT_MODEL."
    )]
    agent_model: Option<String>,

    #[arg(
        long,
        default_value = "SDKCHECK_AGENT_API_KEY",
        help = "Environment variable name that stores the audit agent API key."
    )]
    agent_api_key_env: String,

    #[arg(
        long,
        default_value_t = 12,
        help = "Maximum agent steps before sdkcheck stops the audit."
    )]
    max_steps: u32,
}

pub fn run() -> Result<()> {
    let _ = dotenvy::dotenv();
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(command) => {
            let agent_base_url = command
                .agent_base_url
                .or_else(|| std::env::var("SDKCHECK_AGENT_BASE_URL").ok())
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            let agent_model = command
                .agent_model
                .or_else(|| std::env::var("SDKCHECK_AGENT_MODEL").ok())
                .unwrap_or_default();

            let report = docs::run(DocsAuditOptions {
                backend: command.backend,
                docs: command.docs,
                workspace: command.workspace,
                goal: command.goal,
                success_criteria: command.success_criteria,
                workdir: command.workdir,
                output: command.output.clone(),
                json_output: command.json_output.clone(),
                timeout_seconds: command.timeout_seconds,
                forwarded_env_names: command.forwarded_envs,
                agent_base_url,
                agent_model,
                agent_api_key_env: command.agent_api_key_env,
                max_steps: command.max_steps,
            })?;

            println!("wrote report: {}", command.output.display());
            if let Some(json_output) = &command.json_output {
                println!("wrote json report: {}", json_output.display());
            }
            println!("status: {}", report.status);
            println!("classification: {}", report.classification);

            if !report.status.is_success() {
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
