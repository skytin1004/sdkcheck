use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::recipes::co_op_translator::{self, CoOpTranslatorOptions};
use crate::runner::Backend;

#[derive(Debug, Parser)]
#[command(
    name = "sdkcheck",
    version,
    about = "Turn product documentation into executable QA scenarios."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Run(RunCommand),
}

#[derive(Debug, Args)]
struct RunCommand {
    #[arg(long, value_enum, default_value_t = Recipe::CoOpTranslator)]
    recipe: Recipe,

    #[arg(long, value_enum, default_value_t = Backend::Docker)]
    backend: Backend,

    #[arg(long, default_value = ".sdkcheck-work")]
    workdir: PathBuf,

    #[arg(long, default_value = "reports/co-op-translator.md")]
    output: PathBuf,

    #[arg(long)]
    json_output: Option<PathBuf>,

    #[arg(long, default_value_t = 900)]
    timeout_seconds: u64,

    #[arg(long = "secret")]
    secrets: Vec<String>,

    #[arg(long)]
    live: bool,

    #[arg(long)]
    fake_openai: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Recipe {
    CoOpTranslator,
}

impl std::fmt::Display for Recipe {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CoOpTranslator => write!(formatter, "co-op-translator"),
        }
    }
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run(command) => match command.recipe {
            Recipe::CoOpTranslator => {
                let report = co_op_translator::run(CoOpTranslatorOptions {
                    backend: command.backend,
                    workdir: command.workdir,
                    output: command.output.clone(),
                    json_output: command.json_output.clone(),
                    timeout_seconds: command.timeout_seconds,
                    secret_names: command.secrets,
                    live: command.live,
                    fake_openai: command.fake_openai,
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
        },
    }

    Ok(())
}
