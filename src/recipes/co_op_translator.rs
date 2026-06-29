use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use chrono::Local;

use crate::fake_openai::FakeOpenAiServer;
use crate::models::{FailureClassification, ReportStatus, ScenarioReport};
use crate::report::{write_json_report, write_markdown_report};
use crate::runner::{Backend, CommandRunner, CommandSpec};
use crate::secrets::SecretSet;

const COOP_REPO_URL: &str = "https://github.com/Azure/co-op-translator.git";

#[derive(Debug, Clone)]
pub struct CoOpTranslatorOptions {
    pub backend: Backend,
    pub workdir: PathBuf,
    pub output: PathBuf,
    pub json_output: Option<PathBuf>,
    pub timeout_seconds: u64,
    pub secret_names: Vec<String>,
    pub live: bool,
    pub fake_openai: bool,
}

pub fn run(options: CoOpTranslatorOptions) -> Result<ScenarioReport> {
    let run_dir = create_run_dir(&options.workdir)?;
    CommandRunner::new(options.backend, &run_dir, SecretSet::default())
        .with_timeout_seconds(options.timeout_seconds)
        .prepare()?;
    let docker_venv = DockerVolumeGuard::new(options.backend, &run_dir)?;

    let mut secrets = SecretSet::from_env_names(&options.secret_names);
    let fake_server = if options.fake_openai {
        Some(FakeOpenAiServer::start(options.backend, &run_dir)?)
    } else {
        None
    };

    if let Some(server) = &fake_server {
        secrets.add_value("OPENAI_API_KEY", "sdkcheck-fake-key");
        secrets.add_value("OPENAI_CHAT_MODEL_ID", "sdkcheck-fake-model");
        secrets.add_value("OPENAI_BASE_URL", server.base_url());
    }

    let runner = CommandRunner::new(options.backend, &run_dir, secrets.clone())
        .with_timeout_seconds(options.timeout_seconds)
        .with_docker_network(
            fake_server
                .as_ref()
                .and_then(|server| server.docker_network()),
        )
        .with_docker_venv_volume(docker_venv.name());
    let mut commands = Vec::new();

    let mut can_continue = record_command(
        &mut commands,
        runner.run(
            CommandSpec::new("clone Co-op Translator repo", "git", &run_dir).args([
                "clone",
                "--depth",
                "1",
                COOP_REPO_URL,
                "repo",
            ]),
        ),
    )?;

    if can_continue {
        prepare_fixture(&run_dir)?;
    }

    if can_continue {
        can_continue = record_command(
            &mut commands,
            runner.run(
                CommandSpec::new("create Python virtual environment", "python", &run_dir)
                    .args(["-m", "venv", venv_dir(options.backend)])
                    .envs(common_env(&secrets)),
            ),
        )?;
    }

    if can_continue {
        can_continue = record_command(
            &mut commands,
            runner.run(
                CommandSpec::new("upgrade pip", venv_python(options.backend), &run_dir)
                    .args(["-m", "pip", "install", "--upgrade", "pip"])
                    .envs(common_env(&secrets)),
            ),
        )?;
    }

    if can_continue {
        can_continue = record_command(
            &mut commands,
            runner.run(
                CommandSpec::new(
                    "install Co-op Translator from PyPI",
                    venv_python(options.backend),
                    &run_dir,
                )
                .args(["-m", "pip", "install", "co-op-translator"])
                .envs(common_env(&secrets)),
            ),
        )?;
    }

    if can_continue {
        can_continue = record_command(
            &mut commands,
            runner.run(
                CommandSpec::new(
                    "verify translate CLI",
                    venv_tool("translate", options.backend),
                    &run_dir,
                )
                .args(["--help"])
                .envs(common_env(&secrets)),
            ),
        )?;
    }

    if can_continue {
        can_continue = record_command(
            &mut commands,
            runner.run(
                CommandSpec::new(
                    "verify co-op-review CLI",
                    venv_tool("co-op-review", options.backend),
                    &run_dir,
                )
                .args(["--help"])
                .envs(common_env(&secrets)),
            ),
        )?;
    }

    let provider_ready = provider_ready(&secrets);

    if can_continue {
        can_continue = record_command(
            &mut commands,
            runner.run(
                CommandSpec::new(
                    "run Markdown translation dry-run",
                    venv_tool("translate", options.backend),
                    &run_dir,
                )
                .args(["-l", "ko", "-md", "--dry-run", "-y", "-r", "fixture"])
                .envs(common_env(&secrets)),
            ),
        )?;
    }

    if can_continue && provider_ready && (options.fake_openai || options.live) {
        can_continue = record_command(
            &mut commands,
            runner.run(
                CommandSpec::new(
                    "run live Markdown translation",
                    venv_tool("translate", options.backend),
                    &run_dir,
                )
                .args(["-l", "ko", "-md", "-y", "-r", "fixture"])
                .envs(common_env(&secrets)),
            ),
        )?;
    }

    if can_continue && provider_ready && (options.fake_openai || options.live) {
        let _ = record_command(
            &mut commands,
            runner.run(
                CommandSpec::new(
                    "review generated translation",
                    venv_tool("co-op-review", options.backend),
                    &run_dir,
                )
                .args(["-l", "ko", "-r", "fixture", "--format", "github"])
                .envs(common_env(&secrets)),
            ),
        )?;
    }

    let generated_files = generated_files(&run_dir);
    let expected_files = expected_files(&run_dir);
    let expected_files_exist = expected_files.iter().all(|path| path.exists());
    let command_failed = commands.iter().any(|command| !command.success);
    let passed = !command_failed && (!provider_ready || expected_files_exist);
    let classification = classify(passed, provider_ready, &commands, expected_files_exist);
    let suggestions = suggestions(passed, provider_ready, expected_files_exist, &commands);

    let report = ScenarioReport {
        title: "sdkcheck report: Co-op Translator".to_string(),
        status: if passed {
            ReportStatus::Passed
        } else {
            ReportStatus::Failed
        },
        classification,
        summary: summary(passed, provider_ready, expected_files_exist, &commands),
        backend: options.backend.to_string(),
        run_dir: run_dir.clone(),
        scenario_steps: scenario_steps(),
        docs_observations: docs_observations(&run_dir),
        provided_secrets: secrets.names(),
        missing_secrets: secrets.missing_names(),
        commands,
        generated_files,
        suggestions,
        reproduction: reproduction_command(
            options.backend,
            options.fake_openai,
            &options.output,
            options.json_output.as_deref(),
            options.timeout_seconds,
        ),
    };

    write_markdown_report(&report, &options.output, &secrets)?;
    if let Some(json_output) = &options.json_output {
        write_json_report(&report, json_output, &secrets)?;
    }

    drop(fake_server);

    Ok(report)
}

fn create_run_dir(workdir: &PathBuf) -> Result<PathBuf> {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let base = if workdir.is_absolute() {
        workdir.clone()
    } else {
        std::env::current_dir()
            .context("failed to resolve current directory")?
            .join(workdir)
    };
    let run_dir = base
        .join("runs")
        .join(format!("co-op-translator-{timestamp}"));
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("failed to create run dir `{}`", run_dir.display()))?;
    Ok(run_dir)
}

struct DockerVolumeGuard {
    name: Option<String>,
}

impl DockerVolumeGuard {
    fn new(backend: Backend, run_dir: &std::path::Path) -> Result<Self> {
        if backend != Backend::Docker {
            return Ok(Self { name: None });
        }

        let suffix = docker_safe_name(
            run_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("run"),
        );
        let name = format!("sdkcheck-venv-{suffix}");

        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", &name])
            .output();
        let output = Command::new("docker")
            .args(["volume", "create", &name])
            .output()
            .context("failed to start Docker venv volume creation")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "failed to create Docker venv volume `{}`\nstdout:\n{}\nstderr:\n{}",
                name,
                String::from_utf8_lossy(&output.stdout).trim_end(),
                String::from_utf8_lossy(&output.stderr).trim_end()
            ));
        }

        Ok(Self { name: Some(name) })
    }

    fn name(&self) -> Option<String> {
        self.name.clone()
    }
}

impl Drop for DockerVolumeGuard {
    fn drop(&mut self) {
        if let Some(name) = &self.name {
            let _ = Command::new("docker")
                .args(["volume", "rm", "-f", name])
                .output();
        }
    }
}

fn docker_safe_name(input: &str) -> String {
    let normalized = input
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();

    let trimmed = normalized.trim_matches('-');
    if trimmed.is_empty() {
        "run".to_string()
    } else {
        trimmed.to_string()
    }
}

fn record_command(
    commands: &mut Vec<crate::models::CommandResult>,
    result: Result<crate::models::CommandResult>,
) -> Result<bool> {
    let command = result?;
    let success = command.success;
    commands.push(command);
    Ok(success)
}

fn prepare_fixture(run_dir: &std::path::Path) -> Result<()> {
    let fixture = run_dir.join("fixture");
    fs::create_dir_all(fixture.join("docs"))
        .with_context(|| format!("failed to create fixture `{}`", fixture.display()))?;

    fs::write(
        fixture.join("README.md"),
        "# Sample Project\n\nThis is a sample document for sdkcheck Co-op Translator dogfood.\n\n```python\nprint(\"hello from sdkcheck\")\n```\n\nSee [Setup](docs/setup.md) for details.\n",
    )
    .context("failed to write fixture README")?;

    fs::write(
        fixture.join("docs").join("setup.md"),
        "# Setup\n\nSet the `EXAMPLE_API_KEY` environment variable.\n",
    )
    .context("failed to write fixture setup doc")?;

    Ok(())
}

fn common_env(secrets: &SecretSet) -> BTreeMap<String, String> {
    let mut env = secrets.env_pairs();
    env.insert("PYTHONUTF8".to_string(), "1".to_string());
    env.insert("PYTHONIOENCODING".to_string(), "utf-8".to_string());
    env
}

fn provider_ready(secrets: &SecretSet) -> bool {
    secrets.has_all(&["OPENAI_API_KEY", "OPENAI_CHAT_MODEL_ID"])
        || secrets.has_all(&[
            "AZURE_OPENAI_API_KEY",
            "AZURE_OPENAI_ENDPOINT",
            "AZURE_OPENAI_CHAT_DEPLOYMENT_NAME",
            "AZURE_OPENAI_API_VERSION",
        ])
}

fn venv_dir(backend: Backend) -> &'static str {
    match backend {
        Backend::Docker => "/venv",
        Backend::Local => ".venv",
    }
}

fn venv_python(backend: Backend) -> String {
    match backend {
        Backend::Docker => "/venv/bin/python".to_string(),
        Backend::Local if cfg!(windows) => ".venv\\Scripts\\python.exe".to_string(),
        Backend::Local => ".venv/bin/python".to_string(),
    }
}

fn venv_tool(name: &str, backend: Backend) -> String {
    match backend {
        Backend::Docker => format!("/venv/bin/{name}"),
        Backend::Local if cfg!(windows) => format!(".venv\\Scripts\\{name}.exe"),
        Backend::Local => format!(".venv/bin/{name}"),
    }
}

fn expected_files(run_dir: &std::path::Path) -> Vec<PathBuf> {
    vec![
        run_dir
            .join("fixture")
            .join("translations")
            .join("ko")
            .join("README.md"),
        run_dir
            .join("fixture")
            .join("translations")
            .join("ko")
            .join("docs")
            .join("setup.md"),
    ]
}

fn generated_files(run_dir: &std::path::Path) -> Vec<PathBuf> {
    expected_files(run_dir)
        .into_iter()
        .filter(|path| path.exists())
        .collect()
}

fn docs_observations(run_dir: &std::path::Path) -> Vec<String> {
    let repo = run_dir.join("repo");
    let mut observations = Vec::new();

    let readme = fs::read_to_string(repo.join("README.md")).unwrap_or_default();
    if readme.contains("pip install") || readme.contains("co-op-translator") {
        observations.push("README describes installing or using Co-op Translator.".to_string());
    }

    let cli = fs::read_to_string(repo.join("docs").join("cli.md")).unwrap_or_default();
    if cli.contains("translate") && cli.contains("-md") {
        observations.push(
            "CLI docs describe Markdown translation through `translate -l ... -md`.".to_string(),
        );
    }
    if cli.contains("co-op-review") {
        observations.push(
            "CLI docs describe `co-op-review` for validating generated translations.".to_string(),
        );
    }

    let configuration =
        fs::read_to_string(repo.join("docs").join("configuration.md")).unwrap_or_default();
    if configuration.contains("OPENAI_API_KEY") || configuration.contains("AZURE_OPENAI") {
        observations.push("Configuration docs identify OpenAI or Azure OpenAI credentials as required for live translation.".to_string());
    }

    if observations.is_empty() {
        observations.push("No specific docs observations were extracted.".to_string());
    }

    observations
}

fn scenario_steps() -> Vec<String> {
    vec![
        "Fetch Co-op Translator source and docs.".to_string(),
        "Prepare a minimal Markdown fixture with a link and code block.".to_string(),
        "Create an isolated Python environment.".to_string(),
        "Install Co-op Translator from PyPI.".to_string(),
        "Verify documented CLI entry points.".to_string(),
        "Run Markdown translation dry-run.".to_string(),
        "Run deterministic live translation when credentials or fake OpenAI are available."
            .to_string(),
        "Review generated translations and verify expected files.".to_string(),
    ]
}

fn classify(
    passed: bool,
    provider_ready: bool,
    commands: &[crate::models::CommandResult],
    expected_files_exist: bool,
) -> FailureClassification {
    if passed {
        return FailureClassification::None;
    }

    if !provider_ready {
        return FailureClassification::Environment;
    }

    if commands.iter().any(|command| {
        (!command.success || command.timed_out)
            && (command.label.contains("clone")
                || command.label.contains("virtual environment")
                || command.label.contains("upgrade pip")
                || command.stderr.contains("Network/connection error"))
    }) {
        return FailureClassification::Environment;
    }

    if commands
        .iter()
        .any(|command| !command.success && command.label.contains("install"))
    {
        return FailureClassification::Product;
    }

    if !expected_files_exist && commands.iter().all(|command| command.success) {
        return FailureClassification::Docs;
    }

    if !expected_files_exist {
        return FailureClassification::Product;
    }

    FailureClassification::UnclearScenario
}

fn suggestions(
    passed: bool,
    provider_ready: bool,
    expected_files_exist: bool,
    commands: &[crate::models::CommandResult],
) -> Vec<String> {
    if passed {
        return Vec::new();
    }

    let mut suggestions = Vec::new();

    if let Some(command) = commands.iter().find(|command| !command.success) {
        if command.timed_out {
            suggestions.push(format!(
                "Increase `--timeout-seconds` or inspect why `{}` exceeded the command timeout.",
                command.label
            ));
        } else if command.label.contains("virtual environment")
            || command.label.contains("upgrade pip")
        {
            suggestions.push("Ensure Python 3 is available in the selected backend and can create virtual environments.".to_string());
        } else if command.stderr.contains("Network/connection error") {
            suggestions.push("Check connectivity to the configured OpenAI-compatible endpoint. For local deterministic dogfood, retry with `--fake-openai` and inspect whether the fake endpoint stayed reachable for the full run.".to_string());
        } else if command.label.contains("clone") {
            suggestions.push(
                "Check network access and Git availability for fetching the target repository."
                    .to_string(),
            );
        } else {
            suggestions.push(format!(
                "Inspect the `{}` command evidence and fix the first failing step before continuing the scenario.",
                command.label
            ));
        }
    }

    if !provider_ready {
        suggestions.push("Provide `OPENAI_API_KEY` + `OPENAI_CHAT_MODEL_ID`, a complete Azure OpenAI credential set, or use `--fake-openai` for deterministic local dogfood.".to_string());
    }

    if provider_ready && !expected_files_exist {
        suggestions.push("Clarify the expected translation output paths or fix the product behavior so generated files match the documented workflow.".to_string());
    }

    if suggestions.is_empty() {
        suggestions.push("Inspect the failed command evidence and add a tighter docs or product fix once the failure mode is clear.".to_string());
    }

    suggestions
}

fn summary(
    passed: bool,
    provider_ready: bool,
    expected_files_exist: bool,
    commands: &[crate::models::CommandResult],
) -> String {
    if passed {
        return "Co-op Translator docs produced a runnable scenario. Install, CLI preflight, translation, review, and expected output verification passed.".to_string();
    }

    if let Some(command) = commands.iter().find(|command| !command.success) {
        return format!(
            "The scenario stopped at `{}`. sdkcheck captured the command evidence and classified the run from the first failing step.",
            command.label
        );
    }

    if !provider_ready {
        return "Co-op Translator install and CLI preflight ran, but live translation requires provider credentials. The run is classified as an environment issue.".to_string();
    }

    if !expected_files_exist {
        return "Co-op Translator ran with provider credentials, but expected translation files were not found.".to_string();
    }

    "The scenario failed before sdkcheck could assign a sharper product or docs cause.".to_string()
}

fn reproduction_command(
    backend: Backend,
    fake_openai: bool,
    output: &std::path::Path,
    json_output: Option<&std::path::Path>,
    timeout_seconds: u64,
) -> String {
    let mut command = format!(
        "sdkcheck run --recipe co-op-translator --backend {backend} --output {} --timeout-seconds {timeout_seconds}",
        output.display(),
    );

    if let Some(json_output) = json_output {
        command.push_str(&format!(" --json-output {}", json_output.display()));
    }

    if fake_openai {
        command.push_str(" --fake-openai");
    }

    command
}
