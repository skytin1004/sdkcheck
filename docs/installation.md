# Installation

sdkcheck is currently distributed as a Rust CLI from source and, after the first tagged release, as prebuilt GitHub Release binaries.

## From GitHub Releases

After `v0.1.0` is published, download the archive for your platform from [GitHub Releases](https://github.com/skytin1004/sdkcheck/releases).

Archives are named:

```text
sdkcheck-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
sdkcheck-v0.1.0-x86_64-apple-darwin.tar.gz
sdkcheck-v0.1.0-aarch64-apple-darwin.tar.gz
sdkcheck-v0.1.0-x86_64-pc-windows-msvc.zip
```

Each release includes per-archive `.sha256` files and a combined `SHA256SUMS` file.

## From Git

Install from the public repository:

```bash
cargo install --locked --git https://github.com/skytin1004/sdkcheck.git sdkcheck
```

This builds sdkcheck locally and requires a compatible Rust toolchain.

## From a Local Checkout

Build:

```bash
cargo build --locked
```

Run:

```bash
cargo run --locked -- run --recipe co-op-translator --backend docker --fake-openai
```

## Verify the Install

```bash
sdkcheck --version
sdkcheck run --recipe co-op-translator --backend docker --fake-openai
```

The deterministic dogfood run should finish with:

```text
status: passed
classification: none
```

## Future Package Managers

The first public release will focus on GitHub Release binaries and source installs. Homebrew, Scoop, Chocolatey, cargo-binstall metadata, and installer scripts are good follow-up distribution channels once the release artifact shape has settled.
