![sdkcheck banner](assets/banner.svg)

# sdkcheck

sdkcheck audits whether an agent can actually follow your product docs, install your product, and complete the intended flow.

It does not summarize docs. It runs the scenario the docs describe.

The output is evidence: failing steps, command logs, missing env names, generated files, failure classification, and a reproduction command.

One CLI, one job: audit whether an agent can execute the documented product scenario.

## Why

Docs are part of the runtime contract for agents. Humans compensate for stale commands, missing setup steps, and unstated secrets. Agents do not. sdkcheck makes that gap visible before your users hit it.

## What sdkcheck needs

- docs: a local path or external URL, passed with `--docs`
- goal: the scenario the agent must complete, passed with `--goal`
- credentials: stored in your shell or `.env`, forwarded explicitly with `--env`
- an OpenAI-compatible chat completions endpoint for sdkcheck's audit agent

`.env` is only a place to load values from. sdkcheck forwards only the env names you allowlist with `--env`.

## Install

Until crates.io packaging is ready, install from a checkout:

```bash
git clone https://github.com/skytin1004/sdkcheck.git
cd sdkcheck
cargo install --path . --locked
```

## Quick Start

Create a `.env` with the audit agent configuration and any product credentials the scenario needs:

```dotenv
SDKCHECK_AGENT_API_KEY=...
SDKCHECK_AGENT_MODEL=gpt-4.1-mini

EXAMPLE_API_KEY=...
EXAMPLE_APP_KEY=...
EXAMPLE_SITE=api.example.com
```

Then point sdkcheck at the docs and describe the scenario:

```bash
sdkcheck run \
  --docs https://docs.example.com/api/latest/ \
  --goal "Install the SDK and make one successful example API request." \
  --env EXAMPLE_API_KEY \
  --env EXAMPLE_APP_KEY \
  --env EXAMPLE_SITE \
  --json-output reports/run.json
```

Local docs use the same `--docs` flag:

```bash
sdkcheck run \
  --docs README.md \
  --docs docs/quickstart.md \
  --workspace . \
  --goal "Install the SDK and complete the quickstart." \
  --env ACME_API_KEY
```

If all docs are URLs, sdkcheck starts from an empty isolated workspace. If any doc is a local path, sdkcheck copies the current directory into the isolated workspace by default. Pass `--workspace <DIR>` to choose a different source directory.

## What You Get

```text
wrote report: reports/run.md
wrote json report: reports/run.json
status: passed
classification: none
```

Each report includes:

- the step that failed
- the exact command and working directory
- stdout and stderr logs
- missing env names
- generated files
- a reproduction command

## Safety

- Docker is the default backend.
- Local execution requires explicit opt-in with `--backend local`.
- `.env` files are loaded but not copied into the isolated workspace.
- Forwarded env values are masked in captured output and written reports.

Read [SECURITY.md](SECURITY.md) before auditing untrusted docs, workspaces, or production credentials.

## License

Apache-2.0. See [LICENSE](LICENSE).
