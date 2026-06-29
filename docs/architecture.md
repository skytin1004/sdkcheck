# Architecture

sdkcheck is a Rust CLI that turns a product workflow into command evidence.

The MVP is intentionally small:

- One CLI entry point: `sdkcheck run`.
- One recipe: `co-op-translator`.
- Two backends: Docker and local.
- Two report formats: Markdown and JSON.

## Core Flow

```text
CLI args
  |
  v
Recipe options
  |
  v
Run directory under .sdkcheck-work/
  |
  v
CommandRunner prepares backend
  |
  v
Recipe executes ordered commands
  |
  v
CommandResult evidence
  |
  v
ScenarioReport
  |
  v
Markdown/JSON reports under reports/
```

## Modules

| Module | Responsibility |
| --- | --- |
| `src/main.rs` | Thin process entry point. |
| `src/cli.rs` | CLI parsing and exit behavior. |
| `src/runner.rs` | Local and Docker command execution. |
| `src/secrets.rs` | Explicit secret collection and masking. |
| `src/fake_openai.rs` | Deterministic OpenAI-compatible test endpoint. |
| `src/models.rs` | Report and command evidence data structures. |
| `src/report.rs` | Markdown and JSON report rendering and writing. |
| `src/recipes/co_op_translator.rs` | Co-op Translator dogfood workflow. |

## Docker Backend

Docker is the default backend because sdkcheck runs commands from product workflows and may handle API keys.

The Docker backend:

- Builds `sdkcheck-python-runner:0.1.0` when missing.
- Bind mounts the run directory at `/work`.
- Creates a named Docker volume for the Python virtual environment at `/venv`.
- Runs scenario commands in named containers so timeout cleanup can remove them.
- Applies baseline guardrails: `no-new-privileges`, PID limit, memory limit, and CPU limit.
- Runs the fake OpenAI endpoint as a sidecar container when `--fake-openai` is set.
- Cleans up sdkcheck containers, networks, and venv volumes after the run.

The named volume avoids writing large Python dependency trees through a Windows bind mount, which is slower and can destabilize Docker Desktop under heavy file churn.

## Local Backend

The local backend exists for development and debugging. It executes commands on the host. On Windows, plain `python` commands are routed through `cmd /C` so common Python shims work.

Do not use the local backend for untrusted products or with production credentials.

## Failure Classification

The MVP classification is deliberately simple:

- `none`: scenario passed.
- `environment`: missing credentials, network/setup failure, or backend failure.
- `product`: install or runtime behavior failed after the environment was ready.
- `docs`: commands succeeded but expected documented output was missing.
- `unclear-scenario`: sdkcheck could not assign a sharper cause.

This is evidence labeling, not final truth. Reports should give maintainers enough command evidence to reproduce and refine the classification.

## Command Evidence

Each command records:

- Label.
- Reproduction command.
- Working directory.
- Exit code.
- Timeout state.
- Duration.
- stdout.
- stderr.

stdout and stderr are secret-masked, then truncated to keep reports bounded. Markdown is the default human report. JSON is available through `--json-output` for CI parsing.

## Design Trade-offs

The MVP favors a hardcoded first recipe over a general recipe DSL. That makes the first dogfood scenario real sooner and keeps the execution model visible in Rust.

The cost is that adding a second product currently requires Rust code. A public recipe format should wait until sdkcheck has enough real dogfood runs to know which abstractions are stable.
