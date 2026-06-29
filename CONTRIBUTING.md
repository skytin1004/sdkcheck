# Contributing

Thanks for working on sdkcheck. This project is early, so the best contributions are narrow, evidence-backed changes that improve the Docker-first runner, the Co-op Translator dogfood recipe, report quality, or release readiness.

## Development Setup

Install:

- Rust stable.
- Docker Desktop or another Docker daemon.
- Git.

Build and test:

```bash
cargo build --locked
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --locked
```

Run the deterministic dogfood scenario:

```bash
cargo run --locked -- run --recipe co-op-translator --backend docker --fake-openai
```

Use `--backend local` only for development when Docker is unavailable.

## Pull Request Expectations

Before opening a PR:

- Keep the change scoped to one behavior or one documentation improvement.
- Add or update tests when behavior changes.
- Run formatting, clippy, and tests locally.
- Update docs when CLI flags, report shape, security posture, or workflow behavior changes.
- Do not include real API keys, logs with credentials, or generated report artifacts.

## Security-Sensitive Changes

sdkcheck handles command execution and API keys. Changes in these areas need extra care:

- Secret collection, masking, or report rendering.
- Docker command construction.
- Local backend behavior.
- Network access for fake or real LLM endpoints.
- Files written to run directories or reports.

Prefer changes that reduce the amount of trusted host state sdkcheck touches.

## Documentation Style

Docs should be concrete and runnable. If a command is shown, it should be a command a new contributor can copy into a clean checkout.

Use:

- Tutorials for first-run workflows.
- How-to docs for specific tasks.
- Reference docs for exact CLI flags and report fields.
- Explanation docs for architecture and security trade-offs.

## License

By contributing, you agree that your contributions are licensed under Apache-2.0, matching the project license.
