use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use clap::ValueEnum;
use wait_timeout::ChildExt;

use crate::models::CommandResult;
use crate::secrets::SecretSet;

pub const DEFAULT_DOCKER_IMAGE: &str = "sdkcheck-python-runner:0.1.0";
const DEFAULT_COMMAND_TIMEOUT_SECONDS: u64 = 900;
const MAX_REPORT_OUTPUT_BYTES: usize = 64 * 1024;
const DOCKERFILE: &str = r#"FROM python:3.12-slim

ENV DEBIAN_FRONTEND=noninteractive \
    PYTHONUTF8=1 \
    PYTHONIOENCODING=utf-8 \
    PIP_DISABLE_PIP_VERSION_CHECK=1

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        git \
        libglib2.0-0 \
        libgl1 \
        libgomp1 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /work
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Backend {
    Docker,
    Local,
}

impl std::fmt::Display for Backend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Docker => write!(formatter, "docker"),
            Self::Local => write!(formatter, "local"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}

impl CommandSpec {
    pub fn new(
        label: impl Into<String>,
        program: impl Into<String>,
        cwd: impl Into<PathBuf>,
    ) -> Self {
        Self {
            label: label.into(),
            program: program.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            env: BTreeMap::new(),
        }
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn envs(mut self, env: BTreeMap<String, String>) -> Self {
        self.env.extend(env);
        self
    }
}

#[derive(Debug, Clone)]
pub struct CommandRunner {
    backend: Backend,
    mount_root: PathBuf,
    secrets: SecretSet,
    docker_image: String,
    timeout: Duration,
}

impl CommandRunner {
    pub fn new(backend: Backend, mount_root: impl Into<PathBuf>, secrets: SecretSet) -> Self {
        Self {
            backend,
            mount_root: mount_root.into(),
            secrets,
            docker_image: DEFAULT_DOCKER_IMAGE.to_string(),
            timeout: Duration::from_secs(DEFAULT_COMMAND_TIMEOUT_SECONDS),
        }
    }

    pub fn with_timeout_seconds(mut self, seconds: u64) -> Self {
        self.timeout = Duration::from_secs(seconds.max(1));
        self
    }

    pub fn prepare(&self) -> Result<()> {
        match self.backend {
            Backend::Docker => self.ensure_docker_image(),
            Backend::Local => Ok(()),
        }
    }

    pub fn run(&self, spec: CommandSpec) -> Result<CommandResult> {
        match self.backend {
            Backend::Local => self.run_local(spec),
            Backend::Docker => self.run_docker(spec),
        }
    }

    fn run_local(&self, spec: CommandSpec) -> Result<CommandResult> {
        let mut command = local_command(&spec);
        command.current_dir(&spec.cwd).envs(&spec.env);

        if !uses_windows_command_shell(&spec) {
            command.args(&spec.args);
        }

        self.execute(command, &spec, None)
    }

    fn run_docker(&self, spec: CommandSpec) -> Result<CommandResult> {
        let mut command = Command::new("docker");
        let mount = format!(
            "type=bind,source={},target=/work",
            self.mount_root.display()
        );
        let workdir = docker_workdir(&self.mount_root, &spec.cwd)?;
        let container_name = docker_container_name(&self.mount_root, &spec.label);
        cleanup_docker_container(&container_name);

        command
            .arg("run")
            .arg("--rm")
            .arg("--init")
            .arg("--name")
            .arg(&container_name)
            .arg("--security-opt")
            .arg("no-new-privileges")
            .arg("--pids-limit")
            .arg("512")
            .arg("--memory")
            .arg("4g")
            .arg("--cpus")
            .arg("2")
            .arg("--mount")
            .arg(mount)
            .arg("-w")
            .arg(workdir);
        command.arg("--network").arg("bridge");

        if cfg!(target_os = "linux") {
            command
                .arg("--add-host")
                .arg("host.docker.internal:host-gateway");
        }

        for (name, value) in &spec.env {
            command.env(name, value).arg("-e").arg(name);
        }

        command
            .arg(&self.docker_image)
            .arg(&spec.program)
            .args(&spec.args);

        self.execute(
            command,
            &spec,
            Some(TimeoutCleanup::DockerContainer(container_name)),
        )
    }

    fn ensure_docker_image(&self) -> Result<()> {
        let inspect_status = Command::new("docker")
            .args(["image", "inspect", &self.docker_image])
            .output()
            .context("failed to inspect sdkcheck Docker runner image")?;

        if inspect_status.status.success() {
            return Ok(());
        }

        let build_dir = self.mount_root.join(".sdkcheck-docker");
        fs::create_dir_all(&build_dir).with_context(|| {
            format!(
                "failed to create Docker build directory `{}`",
                build_dir.display()
            )
        })?;
        let dockerfile = build_dir.join("Dockerfile");
        fs::write(&dockerfile, DOCKERFILE).with_context(|| {
            format!(
                "failed to write Docker runner Dockerfile `{}`",
                dockerfile.display()
            )
        })?;

        let output = Command::new("docker")
            .arg("build")
            .arg("--pull")
            .arg("-t")
            .arg(&self.docker_image)
            .arg("-f")
            .arg(&dockerfile)
            .arg(&build_dir)
            .output()
            .context("failed to start Docker runner image build")?;

        if output.status.success() {
            return Ok(());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        Err(anyhow!(
            "failed to build Docker runner image `{}`\nstdout:\n{}\nstderr:\n{}",
            self.docker_image,
            stdout.trim_end(),
            stderr.trim_end()
        ))
    }

    fn execute(
        &self,
        mut command: Command,
        spec: &CommandSpec,
        timeout_cleanup: Option<TimeoutCleanup>,
    ) -> Result<CommandResult> {
        let started = Instant::now();
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .with_context(|| format!("failed to start command `{}`", spec.program))?;
        let stdout_reader = child
            .stdout
            .take()
            .context("failed to capture command stdout")?;
        let stderr_reader = child
            .stderr
            .take()
            .context("failed to capture command stderr")?;
        let stdout_handle = thread::spawn(move || read_all(stdout_reader));
        let stderr_handle = thread::spawn(move || read_all(stderr_reader));

        let mut timed_out = false;
        let status = match child.wait_timeout(self.timeout)? {
            Some(status) => status,
            None => {
                timed_out = true;
                if let Some(cleanup) = timeout_cleanup {
                    cleanup.run();
                }
                let _ = child.kill();
                child
                    .wait()
                    .context("failed to wait for timed out command")?
            }
        };
        let duration_ms = started.elapsed().as_millis();

        let stdout = String::from_utf8_lossy(&join_reader(stdout_handle)).to_string();
        let mut stderr = String::from_utf8_lossy(&join_reader(stderr_handle)).to_string();
        if timed_out {
            stderr.push_str(&format!(
                "\n[sdkcheck] command timed out after {} seconds",
                self.timeout.as_secs()
            ));
        }
        let command_text = command_line(&spec.program, &spec.args);
        let stdout = truncate_for_report(&self.secrets.mask(&stdout));
        let stderr = truncate_for_report(&self.secrets.mask(&stderr));

        Ok(CommandResult {
            label: spec.label.clone(),
            command: self.secrets.mask(&command_text),
            cwd: spec.cwd.clone(),
            exit_code: status.code(),
            success: status.success() && !timed_out,
            timed_out,
            stdout,
            stderr,
            duration_ms,
        })
    }
}

enum TimeoutCleanup {
    DockerContainer(String),
}

impl TimeoutCleanup {
    fn run(self) {
        match self {
            Self::DockerContainer(name) => cleanup_docker_container(&name),
        }
    }
}

fn docker_workdir(mount_root: &Path, cwd: &Path) -> Result<String> {
    let relative = cwd.strip_prefix(mount_root).with_context(|| {
        format!(
            "docker command cwd `{}` is outside mount root `{}`",
            cwd.display(),
            mount_root.display()
        )
    })?;

    if relative.as_os_str().is_empty() {
        return Ok("/work".to_string());
    }

    let path = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");

    if path.contains("..") {
        return Err(anyhow!("refusing docker workdir with parent traversal"));
    }

    Ok(format!("/work/{path}"))
}

fn docker_container_name(mount_root: &Path, label: &str) -> String {
    let run_name = mount_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("run");
    let name = format!(
        "sdkcheck-run-{}-{}",
        docker_safe_name(run_name),
        docker_safe_name(label)
    );

    truncate_docker_name(&name)
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

fn truncate_docker_name(name: &str) -> String {
    const MAX_DOCKER_NAME_LEN: usize = 63;

    if name.len() <= MAX_DOCKER_NAME_LEN {
        return name.to_string();
    }

    name.chars().take(MAX_DOCKER_NAME_LEN).collect()
}

fn cleanup_docker_container(name: &str) {
    let _ = Command::new("docker").args(["rm", "-f", name]).output();
}

fn read_all(mut reader: impl Read) -> Vec<u8> {
    let mut buffer = Vec::new();
    let _ = reader.read_to_end(&mut buffer);
    buffer
}

fn join_reader(handle: thread::JoinHandle<Vec<u8>>) -> Vec<u8> {
    handle.join().unwrap_or_default()
}

fn truncate_for_report(input: &str) -> String {
    if input.len() <= MAX_REPORT_OUTPUT_BYTES {
        return input.to_string();
    }

    let mut end = MAX_REPORT_OUTPUT_BYTES;
    while !input.is_char_boundary(end) {
        end -= 1;
    }

    format!(
        "{}\n[sdkcheck] output truncated to {} bytes for report",
        input[..end].trim_end(),
        MAX_REPORT_OUTPUT_BYTES
    )
}

fn command_line(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().map(|arg| quote_arg(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn local_command(spec: &CommandSpec) -> Command {
    if uses_windows_command_shell(spec) {
        let mut command = Command::new("cmd");
        command
            .arg("/C")
            .arg(command_line(&spec.program, &spec.args));
        return command;
    }

    Command::new(local_program_path(&spec.program, &spec.cwd))
}

fn uses_windows_command_shell(spec: &CommandSpec) -> bool {
    cfg!(windows) && spec.program.eq_ignore_ascii_case("python")
}

fn local_program_path(program: &str, cwd: &Path) -> PathBuf {
    if looks_like_path(program) && Path::new(program).is_relative() {
        cwd.join(program)
    } else {
        PathBuf::from(program)
    }
}

fn looks_like_path(program: &str) -> bool {
    program.starts_with('.')
        || program.contains('/')
        || program.contains('\\')
        || program.contains(std::path::MAIN_SEPARATOR)
}

fn quote_arg(arg: &str) -> String {
    if arg.contains(char::is_whitespace) {
        format!("{arg:?}")
    } else {
        arg.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_REPORT_OUTPUT_BYTES, docker_container_name, truncate_for_report};

    #[test]
    fn truncates_long_report_output() {
        let input = "x".repeat(MAX_REPORT_OUTPUT_BYTES + 1);
        let truncated = truncate_for_report(&input);

        assert!(truncated.contains("output truncated"));
        assert!(truncated.len() < input.len() + 128);
    }

    #[test]
    fn docker_container_names_are_safe_and_bounded() {
        let name = docker_container_name(
            std::path::Path::new("co-op-translator-20260629-114348"),
            "run live Markdown translation with a long label",
        );

        assert!(name.starts_with("sdkcheck-run-"));
        assert!(name.len() <= 63);
        assert!(name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        }));
    }
}
