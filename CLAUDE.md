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
* rust-analyzer needs the `rust-src` component (`rustup component add rust-src`, or your distribution's `rust-src` package).
* Linking GPUI on Linux needs the development packages for xkbcommon, xkbcommon-x11, xcb, fontconfig, freetype and wayland.
* A faster linker such as `mold` is optional and configured per developer in an untracked `.cargo/config.toml`.
* Source lives in `src/` and `crates/`. Do not use `mod.rs` files.
