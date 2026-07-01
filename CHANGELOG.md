# Changelog

All notable sdkcheck changes will be recorded in this file.

The format follows the spirit of Keep a Changelog, and sdkcheck uses semantic versioning once public releases begin.

## [Unreleased]

### Added

- Rust CLI skeleton with `sdkcheck run`.
- Local backend for development.
- Docs-centered audit input model with repeatable `--docs` values for local paths and external URLs.
- Optional `--workspace` copying for local docs and source checkouts.
- Automatic `.env` loading with explicit env forwarding by allowlist.
- Secret pass-through by explicit environment variable name.
- Markdown and JSON report output with command evidence and failure classification.
- Per-command timeout, timeout evidence, and bounded report logs.
- Baseline Docker command guardrails with named containers, `no-new-privileges`, PID limit, memory limit, and CPU limit.
- Tag-based GitHub Release workflow for Linux, macOS, and Windows binaries with SHA256 checksums.
- README positioning and CLI guidance for the agent-audit direction.
- Generic `docs + goal + success criteria + agent provider` audit input model.
- Rig-backed audit agent providers for direct OpenAI and Azure OpenAI.
- First open-source project files: contributing guide, security policy, code of conduct, CI, release workflow, and issue templates.

### Known Gaps

- JUnit/SARIF report formats are not implemented yet.
- Multi-scenario batching and richer policy controls are not implemented yet.
- Configurable Docker resource limit flags are planned hardening work.

## [0.1.0] - Unreleased

Initial public MVP target.
