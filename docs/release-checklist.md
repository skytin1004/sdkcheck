# Release Checklist

Use this checklist before the first public sdkcheck release.

## Repository Setup

- Initialize git.
- Use [github.com/skytin1004/sdkcheck](https://github.com/skytin1004/sdkcheck) as the public repository.
- Use [sdkcheck.com](https://sdkcheck.com) as the public homepage.
- Enable branch protection for the default branch.
- Enable GitHub private vulnerability reporting.
- Enable Dependabot alerts.
- Confirm CI, Docker dogfood, and Security workflows are enabled.
- Confirm the Release workflow can create draft releases from `v*.*.*` tags.
- Confirm GitHub Pages is configured for [sdkcheck.com](https://sdkcheck.com).
- Confirm the Pages workflow publishes `site/` and the `site/CNAME` custom domain is active.
- Confirm `thoughts/`, `.sdkcheck-work/`, `reports/`, `target/`, `dist/`, `.env*`, and logs are ignored.

## Required Checks

Run locally:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --locked
cargo package --locked --list
cargo install cargo-audit --locked
cargo audit
```

Run dogfood:

```bash
cargo run --locked -- run --recipe co-op-translator --backend docker --fake-openai --output reports/co-op-translator-docker.md --json-output reports/co-op-translator-docker.json
```

Confirm:

- Report status is `passed`.
- Failure classification is `none`.
- JSON report is valid and includes command evidence.
- No fake or real secret values appear in the report.
- Docker sdkcheck containers, networks, and volumes are cleaned up after the run.

## First Release Artifacts

Minimum public release:

- Source tag, for example `v0.1.0`.
- GitHub release notes copied from `CHANGELOG.md`.
- Built binaries for Linux, macOS, and Windows.
- SHA256 checksums for uploaded binaries.
- Clear source install instructions.

Recommended later:

- cargo-dist release automation.
- cargo-binstall metadata.
- Homebrew formula.
- Container image for the runner when the runner image becomes stable.

## Release Notes Template

```markdown
# sdkcheck v0.1.0

sdkcheck is a Docker-first Rust CLI that turns product docs into executable QA scenarios.

## Highlights

- Co-op Translator dogfood recipe.
- Docker default backend.
- Fake OpenAI-compatible endpoint for deterministic runs.
- Secret pass-through by explicit environment variable name.
- Markdown and JSON reports with command evidence and failure classification.
- Per-command timeout and bounded report logs.

## Known Gaps

- Recipe authoring is not public yet.
- JUnit/SARIF reports are not implemented yet.
- External LLM gateway flags are planned.
- Configurable Docker resource limit flags are planned.
```
