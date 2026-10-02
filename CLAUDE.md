# CLAUDE.md

DBolt is a free, open-source, Rust-based database IDE built on GPUI, inspired by DataGrip's tooling and Zed's fast native UI. It targets Unix-like systems first; Windows support is planned. Performance and responsiveness are core design principles.

All coding rules, GPUI conventions, and database/performance guidelines live in `.rules`. Follow them.

@.rules

## Commands

* `cargo check` — type-check
* `cargo run` — run the app
* `cargo clippy` / `cargo fmt` — lint / format
* `cargo test` — run tests

## Environment notes

* `gpui` and `gpui_platform` are git dependencies on `zed-industries/zed`; they are not on crates.io (the crates.io `gpui` is an outdated 0.2.x). Expect API differences from older GPUI examples.
* The Rust toolchain is Fedora's system Rust (no `rustup`). rust-analyzer needs the `rust-src` package (`sudo dnf install rust-src`).
* Source lives in `src/`. Do not use `mod.rs` files.
