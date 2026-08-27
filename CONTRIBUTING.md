# Contributing to GIBSON

Thank you for helping with GIBSON.

## Before you start

Discuss large product or architecture changes in an issue first.

Keep Omarchy as the primary target. Preserve safe behaviour on other Arch Wayland systems.

Do not commit Mixkit source audio. Read `assets/sounds/README.md` before you change audio code.

## Check a change

Run these commands before you open a pull request:

```bash
cargo fmt --all -- --check
cargo test --frozen --all-features
cargo clippy --frozen --all-targets --all-features -- -D warnings
cargo build --frozen --release
```

Add tests for new model or parser behaviour. Describe manual visual checks in the pull request.

Keep comments for design reasons, safety rules, or non-obvious platform behaviour. Prefer clear code for routine operations.

## Commit scope

Keep each commit focused. Do not include local settings, generated packages, performance logs, or private audio.

All contributions use the project MIT licence.
