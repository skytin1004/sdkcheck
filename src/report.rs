use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::models::ScenarioReport;
use crate::secrets::SecretSet;

pub fn write_json_report(
    report: &ScenarioReport,
    output: &Path,
    secrets: &SecretSet,
) -> Result<()> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create report directory `{}`", parent.display()))?;
    }

    let rendered = serde_json::to_string_pretty(report).context("failed to render JSON report")?;
    fs::write(output, secrets.mask(&rendered))
        .with_context(|| format!("failed to write JSON report `{}`", output.display()))
}

pub fn write_markdown_report(
    report: &ScenarioReport,
    output: &Path,
    secrets: &SecretSet,
) -> Result<()> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create report directory `{}`", parent.display()))?;
    }

    let rendered = secrets.mask(&render_markdown_report(report));
    fs::write(output, rendered)
        .with_context(|| format!("failed to write report `{}`", output.display()))
}

pub fn render_markdown_report(report: &ScenarioReport) -> String {
    let mut markdown = String::new();

    push_line(&mut markdown, &format!("# {}", report.title));
    push_line(&mut markdown, "");
    push_line(&mut markdown, &format!("- Status: `{}`", report.status));
    push_line(
        &mut markdown,
        &format!("- Failure classification: `{}`", report.classification),
    );
    push_line(&mut markdown, &format!("- Backend: `{}`", report.backend));
    push_line(
        &mut markdown,
        &format!("- Run dir: `{}`", report.run_dir.display()),
    );
    push_line(&mut markdown, "");
    push_line(&mut markdown, "## Summary");
    push_line(&mut markdown, "");
    push_line(&mut markdown, &report.summary);
    push_line(&mut markdown, "");
    push_line(&mut markdown, "## Reproduction");
    push_line(&mut markdown, "");
    push_line(&mut markdown, "```bash");
    push_line(&mut markdown, &report.reproduction);
    push_line(&mut markdown, "```");
    push_line(&mut markdown, "");

    push_list(&mut markdown, "Scenario Steps", &report.scenario_steps);
    push_list(
        &mut markdown,
        "Docs Observations",
        &report.docs_observations,
    );
    push_list(&mut markdown, "Provided Secrets", &report.provided_secrets);
    push_list(&mut markdown, "Missing Secrets", &report.missing_secrets);

    push_line(&mut markdown, "## Commands");
    push_line(&mut markdown, "");

    for command in &report.commands {
        let marker = if command.success { "PASS" } else { "FAIL" };
        push_line(&mut markdown, &format!("### {marker}: {}", command.label));
        push_line(&mut markdown, "");
        push_line(&mut markdown, &format!("- Command: `{}`", command.command));
        push_line(
            &mut markdown,
            &format!("- Cwd: `{}`", command.cwd.display()),
        );
        push_line(
            &mut markdown,
            &format!("- Exit code: `{}`", display_exit_code(command.exit_code)),
        );
        push_line(
            &mut markdown,
            &format!("- Timed out: `{}`", command.timed_out),
        );
        push_line(
            &mut markdown,
            &format!("- Duration: `{} ms`", command.duration_ms),
        );
        push_line(&mut markdown, "");
        push_block(&mut markdown, "stdout", &command.stdout);
        push_block(&mut markdown, "stderr", &command.stderr);
    }

    let generated = report
        .generated_files
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    push_list(&mut markdown, "Generated Files", &generated);
    push_list(&mut markdown, "Suggestions", &report.suggestions);

    markdown
}

fn push_list(markdown: &mut String, title: &str, items: &[String]) {
    push_line(markdown, &format!("## {title}"));
    push_line(markdown, "");

    if items.is_empty() {
        push_line(markdown, "- None");
    } else {
        for item in items {
            push_line(markdown, &format!("- {item}"));
        }
    }

    push_line(markdown, "");
}

fn push_block(markdown: &mut String, title: &str, content: &str) {
    push_line(markdown, &format!("<details><summary>{title}</summary>"));
    push_line(markdown, "");
    push_line(markdown, "```text");

    if content.trim().is_empty() {
        push_line(markdown, "(empty)");
    } else {
        push_line(markdown, content.trim_end());
    }

    push_line(markdown, "```");
    push_line(markdown, "");
    push_line(markdown, "</details>");
    push_line(markdown, "");
}

fn push_line(markdown: &mut String, line: &str) {
    markdown.push_str(line);
    markdown.push('\n');
}

fn display_exit_code(exit_code: Option<i32>) -> String {
    exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal-or-unknown".to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::models::{FailureClassification, ReportStatus, ScenarioReport};

    use super::render_markdown_report;

    #[test]
    fn renders_secret_names_without_values() {
        let report = ScenarioReport {
            title: "sdkcheck report".to_string(),
            status: ReportStatus::Passed,
            classification: FailureClassification::None,
            summary: "ok".to_string(),
            backend: "local".to_string(),
            run_dir: PathBuf::from(".sdkcheck-work/runs/example"),
            scenario_steps: vec![],
            docs_observations: vec![],
            provided_secrets: vec!["OPENAI_API_KEY".to_string()],
            missing_secrets: vec![],
            commands: vec![],
            generated_files: vec![],
            suggestions: vec![],
            reproduction: "sdkcheck run".to_string(),
        };

        let rendered = render_markdown_report(&report);

        assert!(rendered.contains("OPENAI_API_KEY"));
    }
}
