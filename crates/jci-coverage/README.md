# jci-coverage

Source-based Rust code coverage with
[`cargo-llvm-cov`](https://crates.io/crates/cargo-llvm-cov), and upload of the report to
[OtterWise](https://getotterwise.com) for code quality reporting.

[![Crates.io](https://img.shields.io/crates/v/jci-coverage.svg)](https://crates.io/crates/jci-coverage)
[![Documentation](https://docs.rs/jci-coverage/badge.svg)](https://docs.rs/jci-coverage)
[![License](https://img.shields.io/crates/l/jci-coverage.svg)](https://github.com/jerus-org/jci-coverage#license)

## Why

Two independent subcommands, not one:

- **`report`** — orchestrates `cargo llvm-cov` (and, when selected, `cargo-nextest`) to
  write `coverage/lcov.info` and print a terminal summary. Rust-specific; aims to be a
  drop-in replacement for `circleci-toolkit`'s `code_coverage` job.
- **`upload`** — takes a coverage file path and posts it to OtterWise. It never assumes
  `report` produced the file, so it works standalone in a non-Rust project against
  whatever coverage report that project's own build already generates.

Kept separate so the CLI surface can grow additional upload targets (Codecov,
Coveralls) later without reshaping coverage generation.

> **Status:** early (0.0.x). `report` and `upload` are implemented; the
> generated orb is not yet — see [ROADMAP.md](../../ROADMAP.md).

## Runtime prerequisites

`report` **orchestrates the `cargo-llvm-cov` binary as a subprocess** (and
`cargo-nextest` too, when `--runner nextest` is selected) — it does not bundle either.
Both must be on `PATH`:

```bash
cargo binstall cargo-llvm-cov cargo-nextest
rustup component add llvm-tools-preview
```

`upload` has no Rust-specific runtime prerequisites — only a network connection and an
OtterWise repository or organisation token.

## Installation

```bash
cargo binstall jci-coverage
# or
cargo install jci-coverage
```

## Usage

```bash
jci-coverage report                          # writes coverage/lcov.info
jci-coverage upload --file coverage/lcov.info # uploads it to OtterWise
```

See `jci-coverage report --help` / `jci-coverage upload --help` for the full flag
reference.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
