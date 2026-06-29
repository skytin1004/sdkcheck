## Summary

<!-- What changed, and why? -->

## Verification

<!-- Paste the commands you ran. -->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --all-targets --all-features --locked`
- [ ] Docker dogfood run, if runner or audit behavior changed

## Security

- [ ] No real secrets, credentials, private logs, or generated reports are included.
- [ ] Secret handling, Docker execution, and local execution changes are called out explicitly.

## Documentation

- [ ] README/docs updated, or not needed because behavior did not change.
