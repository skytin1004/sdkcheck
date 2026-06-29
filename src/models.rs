use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CommandResult {
    pub label: String,
    pub command: String,
    pub cwd: PathBuf,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioReport {
    pub title: String,
    pub status: ReportStatus,
    pub classification: FailureClassification,
    pub summary: String,
    pub backend: String,
    pub run_dir: PathBuf,
    pub scenario_steps: Vec<String>,
    pub docs_observations: Vec<String>,
    pub provided_secrets: Vec<String>,
    pub missing_secrets: Vec<String>,
    pub commands: Vec<CommandResult>,
    pub generated_files: Vec<PathBuf>,
    pub suggestions: Vec<String>,
    pub reproduction: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReportStatus {
    Passed,
    Failed,
}

impl ReportStatus {
    pub fn is_success(self) -> bool {
        self == Self::Passed
    }
}

impl std::fmt::Display for ReportStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Passed => write!(formatter, "passed"),
            Self::Failed => write!(formatter, "failed"),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureClassification {
    None,
    Docs,
    Product,
    Environment,
    UnclearScenario,
}

impl std::fmt::Display for FailureClassification {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(formatter, "none"),
            Self::Docs => write!(formatter, "docs"),
            Self::Product => write!(formatter, "product"),
            Self::Environment => write!(formatter, "environment"),
            Self::UnclearScenario => write!(formatter, "unclear-scenario"),
        }
    }
}
