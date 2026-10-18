# jci-coverage

Source-based Rust code coverage with
[`cargo-llvm-cov`](https://crates.io/crates/cargo-llvm-cov), and upload of the report to
[OtterWise](https://getotterwise.com) for code quality reporting.

[![Crates.io](https://img.shields.io/crates/v/jci-coverage.svg)](https://crates.io/crates/jci-coverage)
[![Documentation](https://docs.rs/jci-coverage/badge.svg)](https://docs.rs/jci-coverage)
[![License](https://img.shields.io/crates/l/jci-coverage.svg)](https://github.com/jerus-org/jci-coverage#license)

## Why

Three independent subcommands, not one:

- **`report`** — orchestrates `cargo llvm-cov` (and, when selected, `cargo-nextest`) to
  write a coverage report and print a terminal summary. `--format` chooses the report:
  `lcov` (the default) writes `coverage/lcov.info`, `cobertura` writes
  `coverage/cobertura.xml`, and `both` writes both from one test run. Rust-specific;
  aims to be a drop-in replacement for `circleci-toolkit`'s `code_coverage` job.
- **`complexity`** — computes cyclomatic complexity for the Rust functions a Cobertura
  report covers and writes it into the report. `cargo llvm-cov` leaves every complexity at
  zero, so without this step OtterWise shows Complexity and CRAP as 0. It reads the source
  files the report names and needs no other tool.
- **`upload`** — takes a coverage file path and posts it to OtterWise. It never assumes
  `report` produced the file, so it works standalone in a non-Rust project against
  whatever coverage report that project's own build already generates.

Kept separate so the CLI surface can grow additional upload targets (Codecov,
Coveralls) later without reshaping coverage generation.

> **Status:** early (0.0.x). `report` and `upload` are implemented, and the
> `jerus-org/jci-coverage` CircleCI orb is generated from them. The orb is
> published (from 0.0.5) — see [ROADMAP.md](../../ROADMAP.md).

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
jci-coverage report                          # writes coverage/lcov.info (--format lcov)
jci-coverage report --format cobertura       # writes coverage/cobertura.xml
jci-coverage report --format both            # writes both, from one test run
jci-coverage upload --file coverage/lcov.info # uploads it to OtterWise
```

To report complexity and CRAP, annotate a Cobertura report and upload that instead:

```bash
jci-coverage report --format cobertura        # writes coverage/cobertura.xml
jci-coverage complexity                       # adds complexity to it, in place
jci-coverage upload --file coverage/cobertura.xml
```

`complexity` is Rust-only. It computes complexity from Rust source and matches it to the
functions `cargo llvm-cov` names in its Cobertura report, so it refuses (before reading any
source or writing anything) a report that lists a file that is not `.rs`, or that lacks
`cargo llvm-cov`'s per-function entries. A Cobertura report from another language or tool
can still be uploaded as it is with `upload`.

`complexity` counts decisions the way [`rust-code-analysis`](https://github.com/mozilla/rust-code-analysis)
does by default (`--rules rca`): one for each `if`, `for`, `while`, `loop`, match arm,
match-arm guard, `?`, `&&` and `||`, and for the keywords `if`, `for`, `while` and `loop`
inside macro arguments. `--rules mccabe` counts one less per `match` and ignores `?`.
Generic functions appear once per instantiation in the report; they are merged into one
entry by default (`--duplicates keep` leaves them), since OtterWise sums every entry.

Limitations of reading source rather than compiled code:

- A closure inside a macro's arguments (`assert!(v.iter().any(|x| ..))`) is reported by
  `cargo llvm-cov` but cannot be seen by the parser. It is left unmatched (`--unmatched
  keep|one|fail` chooses its value) and its decisions count towards the enclosing function.
- Sources are read relative to `--source-root` (default `.`), not the report's `<sources>`.
  Run from the directory the report was built in, or pass `--source-root`; if nothing
  matches, a warning says so.
- The report is replaced by a new regular file, so a symlink is not followed and the file's
  mode is not kept. Use `--output` to write the annotated report elsewhere.

See `jci-coverage report --help`, `jci-coverage complexity --help` and
`jci-coverage upload --help` for the full flag reference.

## CircleCI orb

The `jerus-org/jci-coverage` orb is generated from this CLI by
[`gen-circleci-orb`](https://github.com/jerus-org/gen-circleci-orb) (source in
[`orb/`](../../orb), config in [`gen-circleci-orb.toml`](../../gen-circleci-orb.toml)).
Its executor image carries `jci-coverage`, `cargo-llvm-cov`, `cargo-nextest` and a
Rust toolchain; `cargo-llvm-cov` adds the `llvm-tools-preview` component itself on
first use in CI.

| Job | Use it for |
|-----|------------|
| `report_and_upload` | Rust projects: generate `coverage/lcov.info` and upload it in one job. The replacement for `circleci-toolkit`'s `code_coverage` job. |
| `report_complexity_and_upload` | Rust projects: as `report_and_upload`, but also adds cyclomatic complexity to the Cobertura report and uploads that, so OtterWise shows Complexity and CRAP. One test run writes both lcov and Cobertura reports. |
| `report` | Generate coverage only, e.g. to persist or store the report yourself. |
| `complexity` | Rust projects: add complexity to a Cobertura report an earlier job produced. |
| `upload` | Any project: upload a coverage file an earlier job produced (use `attach_workspace`). Needs no Rust toolchain. |

```yaml
orbs:
  jci-coverage: jerus-org/jci-coverage@0.0

workflows:
  validation:
    jobs:
      - jci-coverage/report_and_upload
```

To show Complexity and CRAP as well, use `jci-coverage/report_complexity_and_upload` in
the same way. Its steps are fixed (`report --format both`, `complexity`, then `upload` of
`coverage/cobertura.xml`), and it takes the same `package`, `runner`, `nextest_profile`,
`no_all_features` and `log_level` parameters as `report_and_upload`.

Set the OtterWise token as the `OTTERWISE_TOKEN` environment variable in each
project's settings. The `upload` job's `repo_token` parameter exists
because it mirrors the CLI flag, but a token passed that way is stored in your
config and shown on the command line, so don't use it. See
[`orb/src/examples`](../../orb/src/examples) for a non-Rust, upload-only workflow.

Each job runs in the orb's own image, so `report` compiles your crate there. A crate
that needs extra system libraries to build can't use the orb's `report` jobs yet;
generate the report in your own executor and use the `upload` job.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
