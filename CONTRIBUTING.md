# Contributing to Somnium Engine

Thanks for your interest! Somnium is a from-scratch Rust engine and a learning
project — issues, discussion, and PRs are welcome.

## Getting set up

Requirements:
- **Rust 1.88+** (the toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml))
- A **C++ toolchain** for the Jolt physics bridge (MSVC on Windows; clang/gcc elsewhere)

```sh
cargo build --workspace
cargo test  --workspace --lib
cargo run   -p hello_engine        # editor demo
```

## Before opening a PR

- **Format:** run `cargo fmt --all`.
- **Lint:** run `cargo clippy --workspace --all-targets` and address what you reasonably can.
- **Warnings:** CI builds with `-D warnings`, so the workspace must compile warning-free.
- **Tests:** `cargo test --workspace --lib` should pass. Add tests for new
  CPU-side logic (meshing, ECS, terrain/voxel math) where practical — GPU paths
  aren't covered in CI.

## Project conventions

A couple of conventions keep this codebase coherent — please follow them:

1. **Study references, don't copy them.** Somnium reimplements patterns from
   other engines from scratch. When a subsystem is informed by a reference, cite
   the specific file in [`ATTRIBUTION.md`](ATTRIBUTION.md) (and in a source
   comment). Never paste third-party source.
2. **Don't commit `example_repo/`.** It holds local reference repositories and is
   gitignored (see the note in [`README.md`](README.md)). The one tracked
   exception is the vendored Jolt Physics subtree, which the build compiles.
3. **Keep the living docs current.** Architectural changes should be reflected in
   [`context.md`](context.md) (and `ATTRIBUTION.md` if a new reference was used).
4. **Match the surrounding style.** New code should read like the code around it —
   naming, comment density, and idiom.

## A note on AI assistance

This project is developed with AI pair-programming assistance (Claude Code).
Contributions made with similar tooling are perfectly welcome — just hold them to
the same bar: understand the code you submit, make sure it builds and is tested,
and keep the attribution rules above.

## License

By contributing, you agree that your contributions are dual-licensed under
**MIT OR Apache-2.0**, matching the project license.
