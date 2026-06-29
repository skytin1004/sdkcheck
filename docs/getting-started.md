# Getting Started

This guide runs the deterministic Co-op Translator dogfood scenario and produces a Markdown report.

## What You Need

- Rust stable with Cargo.
- Docker Desktop or another Docker daemon.
- Git.
- Network access to GitHub and PyPI.

## Step 1: Build sdkcheck

```bash
cargo build --locked
```

This compiles the Rust CLI into `target/debug/sdkcheck`.

## Step 2: Run the Docker Dogfood Scenario

```bash
cargo run --locked -- run --recipe co-op-translator --backend docker --fake-openai
```

The first Docker run may build the local `sdkcheck-python-runner:0.1.0` image. After that, sdkcheck clones Co-op Translator, creates an isolated Python environment in a Docker volume, installs `co-op-translator`, runs CLI preflights, calls a deterministic fake OpenAI-compatible endpoint, and writes a report.

## Step 3: Read the Result

Expected terminal output:

```text
wrote report: reports/co-op-translator.md
status: passed
classification: none
```

Open the report:

```bash
cat reports/co-op-translator.md
```

On Windows PowerShell:

```powershell
Get-Content reports\co-op-translator.md
```

To also write a machine-readable JSON report:

```bash
cargo run --locked -- run --recipe co-op-translator --backend docker --fake-openai --json-output reports/co-op-translator.json
```

## Running Without Docker

The local backend is a development escape hatch:

```bash
cargo run --locked -- run --recipe co-op-translator --backend local --fake-openai
```

Use this only when you trust the product under test and your local Python environment can create virtual environments.

## Running With Real Credentials

Set your provider variables in the shell, then pass only the names to sdkcheck:

```bash
sdkcheck run \
  --recipe co-op-translator \
  --secret OPENAI_API_KEY \
  --secret OPENAI_CHAT_MODEL_ID \
  --live
```

sdkcheck forwards the named variables to the scenario and masks their values in captured output and reports.

## Troubleshooting

If Docker is not running, `--backend docker` will fail before the scenario starts. Start Docker Desktop or retry with `--backend local`.

If cloning or installing fails, check network access to GitHub and PyPI.

If a live run stops before translation, check that the required provider variables are set and non-empty.

If a command times out, increase `--timeout-seconds` or inspect the first timed-out command in the report.
