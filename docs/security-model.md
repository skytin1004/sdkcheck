# Security Model

sdkcheck executes commands and can pass API keys into those commands. Its security posture should look closer to a CI runner than a static documentation tool.

## Assets

The main assets to protect are:

- Provider API keys and model credentials.
- Source code and docs of the product under test.
- Host filesystem outside the run directory.
- CI logs and generated reports.
- Network access from the runner.

## Trust Boundaries

```text
Host machine
  |
  | starts sdkcheck
  v
Rust CLI
  |
  | explicit secret names
  v
Command runner
  |
  | docker backend by default
  v
Product under test
```

The product under test is not fully trusted. It may execute package install scripts, spawn subprocesses, or print secrets by mistake.

## Current Controls

Docker backend:

- Default backend is `docker`.
- Scenario commands run inside a container.
- Run files are isolated under `.sdkcheck-work/`.
- Python virtual environment uses a Docker volume instead of the host filesystem.
- Fake OpenAI runs in a sidecar container on a private Docker network.
- Scenario containers are named so timeout cleanup can remove them.
- Scenario containers use `no-new-privileges`, PID limit, memory limit, and CPU limit.

Secrets:

- sdkcheck forwards only explicitly named environment variables.
- Reports list secret names, not secret values.
- Captured stdout, stderr, and report content are masked by exact secret value.
- Captured stdout and stderr are truncated before they are written to reports.

Repository hygiene:

- `.sdkcheck-work/` is git-ignored.
- `reports/` is git-ignored.
- `.env*`, `*.log`, `dist/`, and `target/` are git-ignored.
- `thoughts/` is git-ignored for private product planning.

## Known Gaps

Secret masking:

- Current masking is exact string replacement.
- It may not catch encoded, truncated, transformed, or partially printed secrets.

Process control:

- Commands have a built-in timeout, but child process tree cleanup is still best-effort outside Docker.
- Docker CPU, memory, and PID limits exist as fixed defaults but are not configurable yet.
- Network policy is basic: Docker bridge or recipe network.

Recipe trust:

- The MVP has one built-in recipe.
- There is no signed recipe format or policy language.

Report control:

- Markdown and JSON reports are readable evidence, not machine-enforced redaction proof.
- Users should review reports before sharing them outside their organization.

## Recommended Usage

For normal use:

```bash
sdkcheck run --recipe co-op-translator --backend docker --fake-openai
```

For real credentials:

- Use least-privilege provider keys.
- Prefer throwaway or test project keys.
- Pass only the required secret names.
- Review reports before sharing.

Avoid:

- Running untrusted products with `--backend local`.
- Passing production credentials to early recipes.
- Uploading reports from failed runs without checking logs.

## Security Roadmap

High-priority hardening:

- Configurable Docker memory, CPU, PID, and network policy options.
- Full-log artifact capture in the ignored run directory.
- JUnit or SARIF report formats for downstream CI systems.
- Stronger secret redaction patterns.
- External LLM gateway flags that do not require provider-specific secrets in recipe code.
