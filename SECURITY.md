# Security Policy

sdkcheck executes product QA scenarios and can pass API keys into those scenarios. Treat it like a CI runner, not like a passive documentation parser.

## Supported Versions

sdkcheck has not made a stable public release yet.

| Version | Supported |
| --- | --- |
| `main` / pre-alpha | Security reports accepted |
| Published releases | Not available yet |

## Reporting a Vulnerability

Do not open a public issue for vulnerabilities that could expose secrets, bypass isolation, or execute unexpected commands.

Until the public repository has GitHub private vulnerability reporting enabled, contact the maintainer privately and include:

- sdkcheck version or commit.
- Operating system.
- Backend used: `docker` or `local`.
- Minimal reproduction steps.
- Whether any real secrets were exposed.
- Logs with secret values removed.

After GitHub private vulnerability reporting is enabled for [github.com/skytin1004/sdkcheck](https://github.com/skytin1004/sdkcheck), use that path for coordinated security reports.

## Threat Model Summary

sdkcheck currently assumes:

- The user controls which recipe runs.
- The user controls which secret names are passed.
- The Docker daemon is trusted.
- The product under test may be buggy or hostile.
- Reports may be shared, so secret values must not appear in them.

Security boundaries today:

- Docker is the default backend.
- The local backend requires explicit opt-in.
- Secrets are passed only by requested environment variable name.
- Secret values are masked in command output and rendered reports.
- Scenario commands have a per-command timeout.
- Docker scenario commands run with named containers and fixed baseline resource/security limits.
- Run artifacts, reports, local env files, logs, release scratch output, and build output are git-ignored by default.

Known gaps:

- Secret masking is exact-string masking, not semantic data-loss prevention.
- Docker resource limits are fixed defaults and are not configurable yet.
- Local backend timeout cleanup is best-effort for child process trees.
- Markdown and JSON reports are evidence artifacts, not machine-enforced redaction proof.
- The MVP has one built-in recipe and no recipe sandbox policy language yet.

See [docs/security-model.md](docs/security-model.md) for the longer design notes.
