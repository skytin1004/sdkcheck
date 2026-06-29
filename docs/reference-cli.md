# CLI Reference

sdkcheck currently exposes one command: `sdkcheck run`.

## `sdkcheck run`

Runs a QA scenario recipe and writes a report.

```bash
sdkcheck run [OPTIONS]
```

## Options

| Option | Value | Default | Effect |
| --- | --- | --- | --- |
| `--recipe` | `co-op-translator` | `co-op-translator` | Selects the scenario recipe to run. |
| `--backend` | `docker` or `local` | `docker` | Selects the command execution backend. |
| `--workdir` | Path | `.sdkcheck-work` | Base directory for run artifacts. |
| `--output` | Path | `reports/co-op-translator.md` | Markdown report path. |
| `--json-output` | Path | None | Optional JSON report path. |
| `--timeout-seconds` | Positive integer | `900` | Per-command timeout. Values below 1 are coerced to 1 second. |
| `--secret` | Environment variable name | None | Passes one named environment variable into the run. Can be repeated. |
| `--live` | Boolean flag | `false` | Allows live provider-backed translation when credentials are present. |
| `--fake-openai` | Boolean flag | `false` | Starts a deterministic fake OpenAI-compatible endpoint for local dogfood. |

## Exit Codes

| Exit code | Meaning |
| --- | --- |
| `0` | The scenario passed or sdkcheck completed successfully. |
| `1` | sdkcheck failed to start, failed to write a report, or the scenario report status was `failed`. |

## Examples

Run the default Docker recipe with the fake endpoint:

```bash
sdkcheck run --fake-openai
```

Run with explicit output:

```bash
sdkcheck run --backend docker --fake-openai --output reports/co-op-translator-docker.md
```

Run with JSON output for CI parsing:

```bash
sdkcheck run --backend docker --fake-openai --json-output reports/co-op-translator.json
```

Run with a shorter command timeout:

```bash
sdkcheck run --backend docker --fake-openai --timeout-seconds 300
```

Run with OpenAI credentials:

```bash
sdkcheck run \
  --recipe co-op-translator \
  --secret OPENAI_API_KEY \
  --secret OPENAI_CHAT_MODEL_ID \
  --live
```

Run with Azure OpenAI credentials:

```bash
sdkcheck run \
  --recipe co-op-translator \
  --secret AZURE_OPENAI_API_KEY \
  --secret AZURE_OPENAI_ENDPOINT \
  --secret AZURE_OPENAI_CHAT_DEPLOYMENT_NAME \
  --secret AZURE_OPENAI_API_VERSION \
  --live
```

## Current Report Fields

The Markdown and JSON reports include:

- Status.
- Failure classification.
- Backend.
- Run directory.
- Summary.
- Reproduction command.
- Scenario steps.
- Docs observations.
- Provided and missing secret names.
- Command evidence.
- Timeout state per command.
- Generated files.
- Suggestions.

## Related

- [Getting started](getting-started.md)
- [Security model](security-model.md)
