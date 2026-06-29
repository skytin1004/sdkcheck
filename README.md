# sdkcheck

sdkcheck is docs CI for AI-agent-era products.

It turns product documentation into executable QA scenarios, runs those scenarios in an isolated environment, captures command evidence, and classifies the result as a docs, product, environment, or unclear scenario failure.

The first dogfood recipe targets [Azure Co-op Translator](https://github.com/Azure/co-op-translator). sdkcheck fetches the project, reads the relevant docs, installs the published package, verifies the documented CLIs, runs a Markdown translation scenario, and writes a report.

- Website: [sdkcheck.com](https://sdkcheck.com)
- Repository: [github.com/skytin1004/sdkcheck](https://github.com/skytin1004/sdkcheck)

## Status

sdkcheck is a pre-alpha Rust CLI. The current MVP is working and Docker-first, but the public recipe system is intentionally narrow.

Implemented today:

- Rust CLI with `sdkcheck run`.
- Docker backend as the default isolation boundary.
- Local backend as a development escape hatch.
- Co-op Translator dogfood recipe.
- Deterministic fake OpenAI-compatible endpoint for local dogfood.
- Explicit secret pass-through by environment variable name.
- Secret value masking in command output and reports.
- Markdown and JSON report output.
- Per-command timeout with timeout evidence in reports.
- Report log truncation to keep CI artifacts bounded.
- Docker command guardrails: named containers, `no-new-privileges`, PID limit, memory limit, and CPU limit.
- GitHub CI, deterministic Docker dogfood workflow, Dependabot, RustSec audit, and OpenSSF Scorecard workflow.

Not implemented yet:

- General recipe authoring.
- JUnit/SARIF reports for CI systems.
- External `--llm-base-url` CLI flags.
- Configurable Docker resource limit flags.
- Enterprise features.

## Install From Source

Prerequisites:

- Rust stable with Cargo.
- Docker Desktop or another Docker daemon for the default backend.
- Git.

Build:

```bash
cargo build --locked
```

Run the deterministic Docker dogfood scenario:

```bash
cargo run --locked -- run --recipe co-op-translator --backend docker --fake-openai
```

Expected CLI output:

```text
wrote report: reports/co-op-translator.md
status: passed
classification: none
```

Reports are written under `reports/`, which is ignored by git because reports may include execution logs.

## Install From Git

```bash
cargo install --locked --git https://github.com/skytin1004/sdkcheck.git sdkcheck
```

After the first tagged release, prebuilt binaries will be published on [GitHub Releases](https://github.com/skytin1004/sdkcheck/releases). See [Installation](docs/installation.md).

## Quick Usage

Docker is the default backend:

```bash
sdkcheck run --recipe co-op-translator --fake-openai
```

Write both Markdown and JSON reports:

```bash
sdkcheck run \
  --recipe co-op-translator \
  --fake-openai \
  --output reports/co-op-translator.md \
  --json-output reports/co-op-translator.json
```

If Docker is unavailable, use the local backend for development:

```bash
sdkcheck run --recipe co-op-translator --backend local --fake-openai
```

Run with real provider credentials by passing only the environment variable names sdkcheck should forward:

```bash
sdkcheck run \
  --recipe co-op-translator \
  --secret OPENAI_API_KEY \
  --secret OPENAI_CHAT_MODEL_ID \
  --json-output reports/co-op-translator.json \
  --live
```

Co-op Translator live runs accept either:

- `OPENAI_API_KEY` and `OPENAI_CHAT_MODEL_ID`, or
- `AZURE_OPENAI_API_KEY`, `AZURE_OPENAI_ENDPOINT`, `AZURE_OPENAI_CHAT_DEPLOYMENT_NAME`, and `AZURE_OPENAI_API_VERSION`.

## Documentation

- [Installation](docs/installation.md)
- [Getting started](docs/getting-started.md)
- [CLI reference](docs/reference-cli.md)
- [Architecture](docs/architecture.md)
- [Security model](docs/security-model.md)
- [Release checklist](docs/release-checklist.md)

## Reports

Markdown and JSON reports include:

- Scenario steps.
- Docs observations.
- Provided and missing secret names.
- Command evidence with stdout/stderr.
- Timeout state per command.
- Generated files.
- Failure classification.
- Suggested next fixes.
- Reproduction command.

## LLM Endpoint Strategy

sdkcheck standardizes LLM-backed scenarios on OpenAI-compatible endpoints. The MVP already uses that contract for `--fake-openai`, which starts a local `/v1/chat/completions` endpoint and passes these variables to the product under test:

```text
OPENAI_BASE_URL
OPENAI_API_KEY
OPENAI_CHAT_MODEL_ID
```

The next planned layer is explicit external gateway support:

```bash
sdkcheck run \
  --recipe co-op-translator \
  --llm-base-url http://localhost:4000/v1 \
  --llm-model gpt-4.1-mini \
  --llm-api-key-secret ANY_LLM_API_KEY
```

That would allow any-llm, Otari, LiteLLM, OpenRouter, Ollama, Azure-compatible gateways, or internal OpenAI-compatible gateways without embedding provider-specific SDKs in the Rust core.

## Security Posture

sdkcheck executes commands from product QA scenarios. Treat it like a CI runner.

Current defaults:

- Docker is the default backend.
- Local execution is available only when explicitly selected.
- Secrets are opt-in by environment variable name.
- Secret values are masked in captured output and reports.
- Command output is truncated in reports.
- Docker commands run with named containers and baseline resource/security options.
- Run directories and reports are ignored by git.

Read [SECURITY.md](SECURITY.md) and [docs/security-model.md](docs/security-model.md) before running sdkcheck against untrusted repositories or with production credentials.

## License

sdkcheck is open source under [Apache-2.0](LICENSE).

The intended long-term model is:

- Open-source core under Apache-2.0.
- Commercial license for future enterprise features.

Enterprise features are intentionally out of the MVP.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).
