use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, anyhow};
use chrono::Local;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::json;

use crate::models::{AuditReport, CommandResult, FailureClassification, ReportStatus};
use crate::report::{write_json_report, write_markdown_report};
use crate::runner::{Backend, CommandRunner, CommandSpec};
use crate::secrets::SecretSet;

const MAX_DOC_FILE_BYTES: usize = 16 * 1024;
const MAX_DOC_BUNDLE_BYTES: usize = 48 * 1024;
const MAX_FILE_READ_BYTES: usize = 12 * 1024;
const MAX_AGENT_OBSERVATION_BYTES: usize = 4 * 1024;
const MAX_GENERATED_FILES: usize = 64;
const MAX_AGENT_RESPONSE_TOKENS: u64 = 800;

#[derive(Debug, Clone)]
pub struct DocsAuditOptions {
    pub backend: Backend,
    pub repo: String,
    pub docs: Vec<PathBuf>,
    pub goal: String,
    pub success_criteria: Vec<String>,
    pub workdir: PathBuf,
    pub output: PathBuf,
    pub json_output: Option<PathBuf>,
    pub timeout_seconds: u64,
    pub forwarded_env_names: Vec<String>,
    pub agent_base_url: String,
    pub agent_model: String,
    pub agent_api_key_env: String,
    pub max_steps: u32,
}

#[derive(Debug, Clone)]
struct AgentFinish {
    verdict: AgentVerdict,
    classification: FailureClassification,
    summary: String,
    suggestions: Vec<String>,
    missing_envs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum AgentVerdict {
    Passed,
    Failed,
    Inconclusive,
}

#[derive(Debug, Clone)]
struct DocsSelection {
    doc_paths: Vec<PathBuf>,
    bundle: String,
    observations: Vec<String>,
}

struct AgentLoop<'a> {
    options: &'a DocsAuditOptions,
    repo_dir: &'a Path,
    docs_selection: &'a DocsSelection,
    runner: &'a CommandRunner,
    forwarded_envs: &'a SecretSet,
    commands: &'a mut Vec<CommandResult>,
    audit_steps: &'a mut Vec<String>,
    docs_observations: &'a mut Vec<String>,
    agent: &'a AgentClient,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct FileStamp {
    len: u64,
    modified_unix_seconds: u64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum AgentAction {
    ReadFile {
        summary: String,
        path: String,
    },
    RunCommand {
        summary: String,
        label: String,
        program: String,
        #[serde(default)]
        args: Vec<String>,
        cwd: Option<String>,
    },
    Finish {
        summary: String,
        verdict: FinishVerdict,
        classification: FinishClassification,
        #[serde(default)]
        suggestions: Vec<String>,
        #[serde(default)]
        missing_envs: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FinishVerdict {
    Passed,
    Failed,
    Inconclusive,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum FinishClassification {
    None,
    Docs,
    Product,
    Environment,
    UnclearAudit,
}

pub fn run(options: DocsAuditOptions) -> Result<AuditReport> {
    let run_dir = create_run_dir(&options.workdir, &options.repo)?;
    let mut audit_steps = Vec::new();
    let mut docs_observations = Vec::new();
    let mut commands = Vec::new();

    CommandRunner::new(options.backend, &run_dir, SecretSet::default())
        .with_timeout_seconds(options.timeout_seconds)
        .prepare()?;

    let forwarded_envs = SecretSet::from_env_names(&options.forwarded_env_names);
    let runner = CommandRunner::new(options.backend, &run_dir, forwarded_envs.clone())
        .with_timeout_seconds(options.timeout_seconds);
    let clone_runner = CommandRunner::new(Backend::Local, &run_dir, SecretSet::default())
        .with_timeout_seconds(options.timeout_seconds);
    let clone_source = clone_source(&options.repo)?;

    audit_steps.push(format!(
        "Clone audit repository `{}` into the isolated run directory.",
        options.repo
    ));
    let cloned = record_command(
        &mut commands,
        clone_runner.run(
            CommandSpec::new("clone audit repository", "git", &run_dir).args([
                "clone",
                "--depth",
                "1",
                &clone_source,
                "repo",
            ]),
        ),
    )?;

    let repo_dir = run_dir.join("repo");
    let mut finish = None;
    let mut baseline_snapshot = BTreeMap::new();

    if cloned {
        match load_docs_selection(&repo_dir, &options.docs) {
            Ok(selection) => {
                audit_steps.push(format!(
                    "Load seed docs for the agent: {}.",
                    selection
                        .doc_paths
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                docs_observations.extend(selection.observations.iter().cloned());
                baseline_snapshot = snapshot_tree(&repo_dir)?;

                let agent = match AgentClient::new(&options) {
                    Ok(agent) => Some(agent),
                    Err(error) => {
                        finish = Some(AgentFinish {
                            verdict: AgentVerdict::Failed,
                            classification: FailureClassification::Environment,
                            summary: format!(
                                "sdkcheck could not start the audit agent: {error:#}"
                            ),
                            suggestions: vec![
                                "Set a valid audit agent API key, base URL, and model before retrying."
                                    .to_string(),
                            ],
                            missing_envs: Vec::new(),
                        });
                        docs_observations.push(
                            "Audit agent configuration failed before command execution."
                                .to_string(),
                        );
                        None
                    }
                };

                if let Some(agent) = agent {
                    finish = Some(
                        AgentLoop {
                            options: &options,
                            repo_dir: &repo_dir,
                            docs_selection: &selection,
                            runner: &runner,
                            forwarded_envs: &forwarded_envs,
                            commands: &mut commands,
                            audit_steps: &mut audit_steps,
                            docs_observations: &mut docs_observations,
                            agent: &agent,
                        }
                        .run(),
                    );
                }
            }
            Err(error) => {
                docs_observations.push(format!("Failed to load docs: {error:#}"));
                finish = Some(AgentFinish {
                    verdict: AgentVerdict::Failed,
                    classification: FailureClassification::Docs,
                    summary: format!("sdkcheck could not load the requested docs: {error:#}"),
                    suggestions: vec![
                        "Pass valid --docs paths or make sure README.md/docs/*.md exist in the target repository."
                            .to_string(),
                    ],
                    missing_envs: Vec::new(),
                });
            }
        }
    }

    let generated_files = if repo_dir.exists() {
        changed_files(&repo_dir, &baseline_snapshot)?
    } else {
        Vec::new()
    };

    let merged_missing_envs = merged_missing_envs(&forwarded_envs, finish.as_ref());
    let classification =
        classification_for_report(finish.as_ref(), &commands, &merged_missing_envs);
    let summary = summary_for_report(finish.as_ref(), &commands, &merged_missing_envs);
    let suggestions = suggestions_for_report(finish.as_ref(), &commands, &merged_missing_envs);
    let status = match finish
        .as_ref()
        .map(|finish| finish.verdict)
        .unwrap_or(AgentVerdict::Failed)
    {
        AgentVerdict::Passed => ReportStatus::Passed,
        AgentVerdict::Failed | AgentVerdict::Inconclusive => ReportStatus::Failed,
    };

    let report = AuditReport {
        title: format!(
            "sdkcheck audit report: {}",
            audit_target_name(&options.repo)
        ),
        status,
        classification,
        summary,
        backend: options.backend.to_string(),
        run_dir: run_dir.clone(),
        audit_steps,
        docs_observations,
        provided_envs: forwarded_envs.names(),
        missing_envs: merged_missing_envs,
        commands,
        generated_files,
        suggestions,
        reproduction: reproduction_command(&options),
    };

    write_markdown_report(&report, &options.output, &forwarded_envs)?;
    if let Some(json_output) = &options.json_output {
        write_json_report(&report, json_output, &forwarded_envs)?;
    }

    Ok(report)
}

impl AgentLoop<'_> {
    fn run(self) -> AgentFinish {
        let mut observations = vec![
            format!("Audit goal: {}", self.options.goal),
            format!(
                "Seed docs: {}",
                self.docs_selection
                    .doc_paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ];

        if !self.options.success_criteria.is_empty() {
            observations.push(format!(
                "Success criteria:\n- {}",
                self.options.success_criteria.join("\n- ")
            ));
        }

        if !self.forwarded_envs.names().is_empty() {
            observations.push(format!(
                "Forwarded env names available to commands: {}",
                self.forwarded_envs.names().join(", ")
            ));
        }

        if !self.forwarded_envs.missing_names().is_empty() {
            observations.push(format!(
                "Forwarded env names requested but missing on the host: {}",
                self.forwarded_envs.missing_names().join(", ")
            ));
        }

        for step in 1..=self.options.max_steps.max(1) {
            let prompt = build_agent_prompt(
                self.options,
                self.docs_selection,
                &observations,
                step,
                self.repo_dir,
                self.forwarded_envs,
            );

            let action = match self.agent.next_action(&prompt) {
                Ok(action) => action,
                Err(error) => {
                    return AgentFinish {
                        verdict: AgentVerdict::Failed,
                        classification: FailureClassification::Environment,
                        summary: format!(
                            "sdkcheck could not get the next action from the audit agent: {error:#}"
                        ),
                        suggestions: vec![
                            "Verify the audit agent endpoint, API key, and model, then retry the audit."
                                .to_string(),
                        ],
                        missing_envs: Vec::new(),
                    };
                }
            };

            match action {
                AgentAction::ReadFile { summary, path } => {
                    let resolved = match resolve_relative_path(self.repo_dir, &path) {
                        Ok(path) => path,
                        Err(error) => {
                            observations.push(format!(
                                "Step {step}: agent requested an invalid file path `{path}` ({error:#})."
                            ));
                            self.audit_steps.push(format!(
                                "Step {step}: agent requested invalid path `{path}`."
                            ));
                            continue;
                        }
                    };

                    let relative = resolved
                        .strip_prefix(self.repo_dir)
                        .unwrap_or(&resolved)
                        .to_path_buf();
                    let content = match read_text_file(&resolved, MAX_FILE_READ_BYTES) {
                        Ok(content) => content,
                        Err(error) => {
                            observations.push(format!(
                                "Step {step}: failed to read `{}` ({error:#}).",
                                relative.display()
                            ));
                            self.audit_steps.push(format!(
                                "Step {step}: failed to read `{}`.",
                                relative.display()
                            ));
                            continue;
                        }
                    };

                    self.audit_steps.push(format!(
                        "Step {step}: read `{}` ({summary}).",
                        relative.display()
                    ));
                    self.docs_observations
                        .push(format!("Agent read `{}`.", relative.display()));
                    observations.push(format!(
                        "Step {step}: file `{}` content:\n{}",
                        relative.display(),
                        content
                    ));
                }
                AgentAction::RunCommand {
                    summary,
                    label,
                    program,
                    args,
                    cwd,
                } => {
                    let cwd_text = cwd.unwrap_or_else(|| ".".to_string());
                    let resolved_cwd = match resolve_relative_path(self.repo_dir, &cwd_text) {
                        Ok(path) => path,
                        Err(error) => {
                            observations.push(format!(
                                "Step {step}: agent requested invalid cwd `{cwd_text}` ({error:#})."
                            ));
                            self.audit_steps.push(format!(
                                "Step {step}: agent requested invalid cwd `{cwd_text}`."
                            ));
                            continue;
                        }
                    };

                    let command_label = if label.trim().is_empty() {
                        summary.clone()
                    } else {
                        label
                    };
                    self.audit_steps
                        .push(format!("Step {step}: run `{}` ({summary}).", command_label));

                    match self.runner.run(
                        CommandSpec::new(command_label.clone(), program, resolved_cwd)
                            .args(args)
                            .envs(command_env(self.forwarded_envs)),
                    ) {
                        Ok(result) => {
                            observations.push(command_observation(step, &result));
                            self.commands.push(result);
                        }
                        Err(error) => {
                            observations.push(format!(
                                "Step {step}: failed to start `{}` ({error:#}).",
                                command_label
                            ));
                            self.commands.push(CommandResult {
                                label: command_label,
                                command: "[sdkcheck internal failure before command start]"
                                    .to_string(),
                                cwd: self.repo_dir.to_path_buf(),
                                exit_code: None,
                                success: false,
                                timed_out: false,
                                stdout: String::new(),
                                stderr: format!("{error:#}"),
                                duration_ms: 0,
                            });
                        }
                    }
                }
                AgentAction::Finish {
                    summary,
                    verdict,
                    classification,
                    suggestions,
                    missing_envs,
                } => {
                    self.audit_steps.push(format!("Step {step}: finish audit."));
                    return AgentFinish {
                        verdict: match verdict {
                            FinishVerdict::Passed => AgentVerdict::Passed,
                            FinishVerdict::Failed => AgentVerdict::Failed,
                            FinishVerdict::Inconclusive => AgentVerdict::Inconclusive,
                        },
                        classification: match classification {
                            FinishClassification::None => FailureClassification::None,
                            FinishClassification::Docs => FailureClassification::Docs,
                            FinishClassification::Product => FailureClassification::Product,
                            FinishClassification::Environment => FailureClassification::Environment,
                            FinishClassification::UnclearAudit => {
                                FailureClassification::UnclearAudit
                            }
                        },
                        summary,
                        suggestions,
                        missing_envs,
                    };
                }
            }
        }

        AgentFinish {
            verdict: AgentVerdict::Inconclusive,
            classification: FailureClassification::UnclearAudit,
            summary: format!(
                "sdkcheck stopped after {} agent steps without a final verdict.",
                self.options.max_steps.max(1)
            ),
            suggestions: vec![
                "Increase --max-steps if the docs require a longer setup path.".to_string(),
                "Tighten --docs and --success so the agent has a narrower target.".to_string(),
            ],
            missing_envs: Vec::new(),
        }
    }
}

fn build_agent_prompt(
    options: &DocsAuditOptions,
    docs_selection: &DocsSelection,
    observations: &[String],
    step: u32,
    repo_dir: &Path,
    forwarded_envs: &SecretSet,
) -> String {
    let success_criteria = if options.success_criteria.is_empty() {
        "- No explicit success criteria were provided. Use the goal and docs to decide whether the intended flow completed."
            .to_string()
    } else {
        format!("- {}", options.success_criteria.join("\n- "))
    };

    let env_names = if options.forwarded_env_names.is_empty() {
        "(none)".to_string()
    } else {
        options.forwarded_env_names.join(", ")
    };
    let missing_env_names = if forwarded_envs.missing_names().is_empty() {
        "(none)".to_string()
    } else {
        forwarded_envs.missing_names().join(", ")
    };

    format!(
        "You are sdkcheck's audit agent.\n\
         Audit step: {step}\n\
         Repository root: {repo_root}\n\
         Goal: {goal}\n\
         Success criteria:\n{success_criteria}\n\
         Forwarded env names available to commands: {env_names}\n\
         Forwarded env names requested but missing on the host: {missing_env_names}\n\
         Seed docs loaded for you:\n{doc_bundle}\n\
         Observations so far:\n- {observations}\n\
         Return exactly one JSON object and nothing else.\n\
         Allowed actions:\n\
         1. {{\"kind\":\"read_file\",\"summary\":\"why you need it\",\"path\":\"relative/path/from/repo\"}}\n\
         2. {{\"kind\":\"run_command\",\"summary\":\"why you are running it\",\"label\":\"short human label\",\"program\":\"python\",\"args\":[\"-m\",\"pip\",\"install\",\".\"],\"cwd\":\"relative/path/from/repo\"}}\n\
         3. {{\"kind\":\"finish\",\"summary\":\"final verdict summary\",\"verdict\":\"passed|failed|inconclusive\",\"classification\":\"none|docs|product|environment|unclear-audit\",\"suggestions\":[\"next action\"],\"missing_envs\":[\"ENV_NAME\"]}}\n\
         Rules:\n\
         - Do not use markdown fences.\n\
         - Do not use shell operators such as &&, ||, ;, |, >, or <.\n\
         - Use explicit programs and args.\n\
         - Keep cwd inside the repository root.\n\
         - Read files before guessing when the docs are unclear.\n\
         - Finish with `environment` if missing credentials or host prerequisites block the documented flow.\n\
         - Finish with `docs` if the documented flow is wrong or incomplete.\n\
         - Finish with `product` if the docs look reasonable but the product behavior is broken.\n\
         - Finish with `none` only when the goal is actually complete.\n",
        repo_root = repo_dir.display(),
        goal = options.goal,
        success_criteria = success_criteria,
        env_names = env_names,
        missing_env_names = missing_env_names,
        doc_bundle = docs_selection.bundle,
        observations = recent_observations(observations).join("\n- "),
    )
}

fn load_docs_selection(repo_dir: &Path, requested_docs: &[PathBuf]) -> Result<DocsSelection> {
    let doc_paths = if requested_docs.is_empty() {
        auto_discover_docs(repo_dir)?
    } else {
        requested_docs
            .iter()
            .map(|path| {
                let resolved = resolve_relative_path(repo_dir, path)?;
                resolved
                    .strip_prefix(repo_dir)
                    .map(|path| path.to_path_buf())
                    .map_err(|error| anyhow!(error))
            })
            .collect::<Result<Vec<_>>>()?
    };

    if doc_paths.is_empty() {
        return Err(anyhow!(
            "no docs were found; pass --docs explicitly or make sure README.md/docs/*.md exist"
        ));
    }

    let mut bundle = String::new();
    let mut total_bytes = 0;
    for path in &doc_paths {
        let content = read_text_file(&repo_dir.join(path), MAX_DOC_FILE_BYTES)?;
        let chunk = format!("FILE: {}\n```text\n{}\n```\n\n", path.display(), content);

        if total_bytes + chunk.len() > MAX_DOC_BUNDLE_BYTES {
            break;
        }

        total_bytes += chunk.len();
        bundle.push_str(&chunk);
    }

    let observations = vec![format!(
        "Loaded {} doc file(s): {}.",
        doc_paths.len(),
        doc_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )];

    Ok(DocsSelection {
        doc_paths,
        bundle,
        observations,
    })
}

fn auto_discover_docs(repo_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut docs = Vec::new();
    let readme = repo_dir.join("README.md");
    if readme.exists() {
        docs.push(PathBuf::from("README.md"));
    }

    let docs_dir = repo_dir.join("docs");
    if docs_dir.is_dir() {
        collect_markdown_files(&docs_dir, repo_dir, &mut docs)?;
    }

    if docs.is_empty() {
        collect_markdown_files(repo_dir, repo_dir, &mut docs)?;
    }

    docs.sort();
    docs.dedup();
    Ok(docs)
}

fn collect_markdown_files(dir: &Path, repo_dir: &Path, docs: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read directory `{}`", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            if should_skip_dir(&entry.file_name().to_string_lossy()) {
                continue;
            }
            collect_markdown_files(&path, repo_dir, docs)?;
            continue;
        }

        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            docs.push(
                path.strip_prefix(repo_dir)
                    .with_context(|| {
                        format!(
                            "failed to make doc path `{}` relative to `{}`",
                            path.display(),
                            repo_dir.display()
                        )
                    })?
                    .to_path_buf(),
            );
        }
    }

    Ok(())
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | ".venv" | "__pycache__" | "node_modules" | "target" | "dist"
    )
}

fn create_run_dir(workdir: &Path, repo: &str) -> Result<PathBuf> {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let base = if workdir.is_absolute() {
        workdir.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to resolve current directory")?
            .join(workdir)
    };
    let run_dir = base
        .join("runs")
        .join(format!("{}-{timestamp}", audit_target_name(repo)));
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("failed to create run dir `{}`", run_dir.display()))?;
    Ok(run_dir)
}

fn audit_target_name(repo: &str) -> String {
    if let Some(local_name) = local_repo_name(repo) {
        return docker_safe_name_like(&local_name);
    }

    let raw = repo
        .trim_end_matches('/')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("audit");
    let raw = raw.strip_suffix(".git").unwrap_or(raw);
    docker_safe_name_like(raw)
}

fn resolve_relative_path(root: &Path, relative: impl AsRef<Path>) -> Result<PathBuf> {
    let relative = relative.as_ref();
    if relative.is_absolute() {
        return Err(anyhow!("absolute paths are not allowed"));
    }

    let mut cleaned = PathBuf::new();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => cleaned.push(part),
            Component::ParentDir => return Err(anyhow!("parent traversal is not allowed")),
            Component::Prefix(_) | Component::RootDir => {
                return Err(anyhow!("invalid path component"));
            }
        }
    }

    Ok(root.join(cleaned))
}

fn clone_source(repo: &str) -> Result<String> {
    let path = Path::new(repo);
    if path.exists() {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .context("failed to resolve current directory for local repository")?
                .join(path)
        };
        let canonical = fs::canonicalize(&absolute).unwrap_or(absolute);
        return Ok(normalize_host_path_for_git(&canonical));
    }

    Ok(repo.to_string())
}

fn normalize_host_path_for_git(path: &Path) -> String {
    let rendered = path.display().to_string();
    rendered
        .strip_prefix(r"\\?\")
        .unwrap_or(&rendered)
        .to_string()
}

fn local_repo_name(repo: &str) -> Option<String> {
    let path = Path::new(repo);
    if !path.exists() {
        return None;
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let absolute = fs::canonicalize(&absolute).unwrap_or(absolute);

    absolute
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
}

fn docker_safe_name_like(input: &str) -> String {
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
        "audit".to_string()
    } else {
        trimmed.to_string()
    }
}

fn read_text_file(path: &Path, max_bytes: usize) -> Result<String> {
    let bytes =
        fs::read(path).with_context(|| format!("failed to read file `{}`", path.display()))?;
    let content = String::from_utf8_lossy(&bytes);
    Ok(truncate_text(&content, max_bytes))
}

fn truncate_text(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }

    let mut end = max_bytes;
    while !input.is_char_boundary(end) {
        end -= 1;
    }

    format!("{}\n[sdkcheck] content truncated", input[..end].trim_end())
}

fn snapshot_tree(root: &Path) -> Result<BTreeMap<PathBuf, FileStamp>> {
    let mut snapshot = BTreeMap::new();
    snapshot_tree_inner(root, root, &mut snapshot)?;
    Ok(snapshot)
}

fn snapshot_tree_inner(
    root: &Path,
    current: &Path,
    snapshot: &mut BTreeMap<PathBuf, FileStamp>,
) -> Result<()> {
    for entry in fs::read_dir(current)
        .with_context(|| format!("failed to read directory `{}`", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            if should_skip_dir(&entry.file_name().to_string_lossy()) {
                continue;
            }
            snapshot_tree_inner(root, &path, snapshot)?;
            continue;
        }

        let metadata = entry.metadata()?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or(0);

        snapshot.insert(
            path.strip_prefix(root)
                .with_context(|| {
                    format!(
                        "failed to make path `{}` relative to `{}`",
                        path.display(),
                        root.display()
                    )
                })?
                .to_path_buf(),
            FileStamp {
                len: metadata.len(),
                modified_unix_seconds: modified,
            },
        );
    }

    Ok(())
}

fn changed_files(root: &Path, baseline: &BTreeMap<PathBuf, FileStamp>) -> Result<Vec<PathBuf>> {
    let current = snapshot_tree(root)?;
    let mut changed = current
        .into_iter()
        .filter_map(|(path, stamp)| match baseline.get(&path) {
            Some(before) if before == &stamp => None,
            _ => Some(path),
        })
        .take(MAX_GENERATED_FILES)
        .collect::<Vec<_>>();
    changed.sort();
    Ok(changed)
}

fn recent_observations(observations: &[String]) -> Vec<String> {
    observations
        .iter()
        .rev()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn command_env(forwarded_envs: &SecretSet) -> BTreeMap<String, String> {
    let mut env = forwarded_envs.env_pairs();
    env.insert("CI".to_string(), "1".to_string());
    env.insert("PYTHONUTF8".to_string(), "1".to_string());
    env.insert("PYTHONIOENCODING".to_string(), "utf-8".to_string());
    env
}

fn command_observation(step: u32, result: &CommandResult) -> String {
    format!(
        "Step {step}: command `{}` finished. success={}, exit_code={:?}, timed_out={}. stdout:\n{}\nstderr:\n{}",
        result.label,
        result.success,
        result.exit_code,
        result.timed_out,
        truncate_text(&result.stdout, MAX_AGENT_OBSERVATION_BYTES),
        truncate_text(&result.stderr, MAX_AGENT_OBSERVATION_BYTES),
    )
}

fn record_command(
    commands: &mut Vec<CommandResult>,
    result: Result<CommandResult>,
) -> Result<bool> {
    let command = result?;
    let success = command.success;
    commands.push(command);
    Ok(success)
}

fn merged_missing_envs(forwarded_envs: &SecretSet, finish: Option<&AgentFinish>) -> Vec<String> {
    let mut names = BTreeSet::new();
    for name in forwarded_envs.missing_names() {
        names.insert(name);
    }
    if let Some(finish) = finish {
        for name in &finish.missing_envs {
            if !name.trim().is_empty() {
                names.insert(name.clone());
            }
        }
    }
    names.into_iter().collect()
}

fn classification_for_report(
    finish: Option<&AgentFinish>,
    commands: &[CommandResult],
    missing_envs: &[String],
) -> FailureClassification {
    if let Some(finish) = finish {
        if finish.verdict == AgentVerdict::Passed {
            return FailureClassification::None;
        }
        if finish.classification == FailureClassification::None {
            return if !missing_envs.is_empty() {
                FailureClassification::Environment
            } else {
                FailureClassification::UnclearAudit
            };
        }
        return finish.classification;
    }

    if !missing_envs.is_empty() {
        return FailureClassification::Environment;
    }

    if let Some(command) = commands.iter().find(|command| !command.success) {
        if command.label.contains("clone") {
            return FailureClassification::Environment;
        }
        return FailureClassification::Product;
    }

    FailureClassification::UnclearAudit
}

fn summary_for_report(
    finish: Option<&AgentFinish>,
    commands: &[CommandResult],
    missing_envs: &[String],
) -> String {
    if let Some(finish) = finish {
        return finish.summary.clone();
    }

    if let Some(command) = commands.iter().find(|command| !command.success) {
        return format!(
            "The audit stopped at `{}` before the agent could reach a final verdict.",
            command.label
        );
    }

    if !missing_envs.is_empty() {
        return format!(
            "The audit did not run because required env names were missing: {}.",
            missing_envs.join(", ")
        );
    }

    "The audit did not produce a final verdict.".to_string()
}

fn suggestions_for_report(
    finish: Option<&AgentFinish>,
    commands: &[CommandResult],
    missing_envs: &[String],
) -> Vec<String> {
    if let Some(finish) = finish {
        if !finish.suggestions.is_empty() {
            return finish.suggestions.clone();
        }
    }

    let mut suggestions = Vec::new();

    if !missing_envs.is_empty() {
        suggestions.push(format!(
            "Provide the missing env names before retrying the audit: {}.",
            missing_envs.join(", ")
        ));
    }

    if let Some(command) = commands.iter().find(|command| !command.success) {
        suggestions.push(format!(
            "Inspect the `{}` command evidence and fix the first failing step before retrying the audit.",
            command.label
        ));
    }

    if suggestions.is_empty() {
        suggestions.push(
            "Tighten the docs selection, goal, or success criteria so the agent can produce a sharper verdict."
                .to_string(),
        );
    }

    suggestions
}

fn reproduction_command(options: &DocsAuditOptions) -> String {
    let mut command = vec![
        "sdkcheck".to_string(),
        "run".to_string(),
        "--repo".to_string(),
        shell_quote(&options.repo),
        "--goal".to_string(),
        shell_quote(&options.goal),
        "--backend".to_string(),
        options.backend.to_string(),
        "--output".to_string(),
        shell_quote(&options.output.display().to_string()),
        "--timeout-seconds".to_string(),
        options.timeout_seconds.to_string(),
        "--agent-base-url".to_string(),
        shell_quote(&options.agent_base_url),
        "--agent-api-key-env".to_string(),
        options.agent_api_key_env.clone(),
        "--max-steps".to_string(),
        options.max_steps.to_string(),
    ];

    if !options.agent_model.trim().is_empty() {
        command.push("--agent-model".to_string());
        command.push(shell_quote(&options.agent_model));
    }

    if let Some(json_output) = &options.json_output {
        command.push("--json-output".to_string());
        command.push(shell_quote(&json_output.display().to_string()));
    }

    for path in &options.docs {
        command.push("--docs".to_string());
        command.push(shell_quote(&path.display().to_string()));
    }

    for criterion in &options.success_criteria {
        command.push("--success".to_string());
        command.push(shell_quote(criterion));
    }

    for env_name in &options.forwarded_env_names {
        command.push("--env".to_string());
        command.push(env_name.clone());
    }

    command.join(" ")
}

fn shell_quote(value: &str) -> String {
    if value.contains(char::is_whitespace) {
        format!("{value:?}")
    } else {
        value.to_string()
    }
}

struct AgentClient {
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl AgentClient {
    fn new(options: &DocsAuditOptions) -> Result<Self> {
        if options.agent_model.trim().is_empty() {
            return Err(anyhow!(
                "missing audit agent model; pass --agent-model or set SDKCHECK_AGENT_MODEL"
            ));
        }

        let api_key = std::env::var(&options.agent_api_key_env).with_context(|| {
            format!(
                "missing audit agent API key in `{}`",
                options.agent_api_key_env
            )
        })?;

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .context("failed to build audit agent HTTP client")?;

        Ok(Self {
            client,
            base_url: options.agent_base_url.trim_end_matches('/').to_string(),
            api_key,
            model: options.agent_model.clone(),
        })
    }

    fn next_action(&self, prompt: &str) -> Result<AgentAction> {
        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&json!({
                "model": self.model,
                "temperature": 0,
                "max_tokens": MAX_AGENT_RESPONSE_TOKENS,
                "messages": [
                    {
                        "role": "system",
                        "content": "You are sdkcheck's audit agent. Return one JSON object only."
                    },
                    {
                        "role": "user",
                        "content": prompt
                    }
                ]
            }))
            .send()
            .context("failed to contact the audit agent endpoint")?;

        let status = response.status();
        let body = response
            .text()
            .context("failed to read audit agent response")?;
        if !status.is_success() {
            return Err(anyhow!(
                "audit agent endpoint returned HTTP {}:\n{}",
                status,
                body
            ));
        }

        let content = extract_message_content(&body)?;
        let json_body = extract_json_object(&content)?;
        serde_json::from_str::<AgentAction>(&json_body)
            .with_context(|| format!("failed to parse audit agent action:\n{}", content.trim()))
    }
}

fn extract_message_content(body: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct ChatCompletionResponse {
        choices: Vec<Choice>,
    }

    #[derive(Deserialize)]
    struct Choice {
        message: Message,
    }

    #[derive(Deserialize)]
    struct Message {
        content: Option<String>,
    }

    let response: ChatCompletionResponse =
        serde_json::from_str(body).context("failed to parse audit agent response payload")?;
    let message = response
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message.content)
        .ok_or_else(|| anyhow!("audit agent response did not include a message content"))?;
    Ok(message)
}

fn extract_json_object(content: &str) -> Result<String> {
    let start = content
        .find('{')
        .ok_or_else(|| anyhow!("audit agent response did not contain JSON"))?;
    let end = content
        .rfind('}')
        .ok_or_else(|| anyhow!("audit agent response did not contain a closing JSON brace"))?;
    Ok(content[start..=end].to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::models::{FailureClassification, ReportStatus};
    use crate::runner::Backend;

    use super::{DocsAuditOptions, audit_target_name, extract_json_object, run, truncate_text};

    #[test]
    fn extracts_json_from_wrapped_response() {
        let extracted = extract_json_object("```json\n{\"kind\":\"finish\"}\n```").expect("json");

        assert_eq!(extracted, "{\"kind\":\"finish\"}");
    }

    #[test]
    fn target_name_strips_git_suffix() {
        assert_eq!(
            audit_target_name("https://github.com/acme/demo.git"),
            "demo"
        );
    }

    #[test]
    fn truncation_keeps_boundary_and_marker() {
        let text = "hello world";
        let truncated = truncate_text(text, 5);

        assert!(truncated.starts_with("hello"));
        assert!(truncated.contains("truncated"));
    }

    #[test]
    fn runs_a_generic_docs_audit_with_a_mock_agent() {
        let temp_root = unique_temp_dir("sdkcheck-audit-test");
        let repo_dir = temp_root.join("repo-source");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        fs::write(
            repo_dir.join("README.md"),
            "# Demo\n\nRun `git status --short` to verify the repo is clean.\n",
        )
        .expect("write readme");

        run_git(&repo_dir, &["init", "--initial-branch=main"]);
        run_git(&repo_dir, &["config", "user.email", "test@example.com"]);
        run_git(&repo_dir, &["config", "user.name", "sdkcheck test"]);
        run_git(&repo_dir, &["add", "README.md"]);
        run_git(&repo_dir, &["commit", "-m", "init"]);

        let server = MockAgentServer::start();
        let workdir = temp_root.join("work");
        let output = temp_root.join("report.md");
        let json_output = temp_root.join("report.json");

        unsafe {
            std::env::set_var("SDKCHECK_TEST_AGENT_KEY", "sdkcheck-test-key");
        }

        let report = run(DocsAuditOptions {
            backend: Backend::Local,
            repo: repo_dir.display().to_string(),
            docs: vec![PathBuf::from("README.md")],
            goal: "Verify the documented git command works.".to_string(),
            success_criteria: vec!["`git status --short` exits with status 0.".to_string()],
            workdir,
            output,
            json_output: Some(json_output),
            timeout_seconds: 120,
            forwarded_env_names: Vec::new(),
            agent_base_url: server.base_url(),
            agent_model: "sdkcheck-test-model".to_string(),
            agent_api_key_env: "SDKCHECK_TEST_AGENT_KEY".to_string(),
            max_steps: 4,
        })
        .expect("audit report");

        unsafe {
            std::env::remove_var("SDKCHECK_TEST_AGENT_KEY");
        }

        assert_eq!(report.status, ReportStatus::Passed);
        assert_eq!(report.classification, FailureClassification::None);
        assert_eq!(report.commands.len(), 2);
        assert!(report.commands.iter().all(|command| command.success));
        assert!(
            report
                .docs_observations
                .iter()
                .any(|entry| entry.contains("README.md"))
        );
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{timestamp}"));
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn run_git(cwd: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    struct MockAgentServer {
        base_url: String,
        _handle: thread::JoinHandle<()>,
    }

    impl MockAgentServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
            let port = listener.local_addr().expect("local addr").port();
            let counter = Arc::new(AtomicUsize::new(0));
            let thread_counter = Arc::clone(&counter);

            let handle = thread::spawn(move || {
                for stream in listener.incoming().take(2) {
                    let mut stream = stream.expect("stream");
                    let mut buffer = [0_u8; 8192];
                    let _ = stream.read(&mut buffer);

                    let step = thread_counter.fetch_add(1, Ordering::SeqCst);
                    let content = if step == 0 {
                        "{\"kind\":\"run_command\",\"summary\":\"verify the documented command\",\"label\":\"verify documented git command\",\"program\":\"git\",\"args\":[\"status\",\"--short\"],\"cwd\":\".\"}"
                    } else {
                        "{\"kind\":\"finish\",\"summary\":\"The documented command completed successfully.\",\"verdict\":\"passed\",\"classification\":\"none\",\"suggestions\":[],\"missing_envs\":[]}"
                    };
                    let body = format!(
                        "{{\"choices\":[{{\"message\":{{\"content\":\"{}\"}}}}]}}",
                        content.replace('\"', "\\\"")
                    );
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write response");
                }
            });

            Self {
                base_url: format!("http://127.0.0.1:{port}/v1"),
                _handle: handle,
            }
        }

        fn base_url(&self) -> String {
            self.base_url.clone()
        }
    }
}
