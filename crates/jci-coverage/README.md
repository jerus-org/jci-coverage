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
  `coverage/cobertura.xml`, and `both` writes both from one test run. Rust-specific.
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

### Risky functions

A per-line average can hide a few very risky functions among many simple ones, so
`complexity` also scores each function on its own: its cyclomatic complexity, and its CRAP
score, `complexity² × (1 − coverage)³ + complexity`, where coverage is the covered share of
that function's own lines (a line belongs to the innermost function or closure containing
it). It logs a warning for each function at or over `--warn-complexity` (default 10) or
`--warn-crap` (default 30), worst first, and ends its summary with the counts. `0` turns a
threshold off. Test code is left out unless `--gate-tests` is set. A function with no
executable lines is judged on complexity alone. Warnings never change the exit code.

To fail the build instead, set `--max-complexity` or `--max-crap` (both off by default). The
report is written first (not with `--dry-run`, which still fails), then `complexity` exits
non-zero. A function over a maximum is reported only as a failure, not also as a warning, and
a function with no executable lines has no CRAP score, so it can only fail `--max-complexity`.

The failure names every function that fails (none are left out), in a table:

```text
6 function(s) fail the gate: 5 over the complexity limit (15), 2 over the CRAP limit (30); 1 of the CRAP failures can be fixed with tests alone.

| Function            | Location                                            | Complexity (max 15) | Covered      | CRAP (max 30) | Over                          | Fix                         |
|---------------------|-----------------------------------------------------|---------------------|--------------|---------------|-------------------------------|-----------------------------|
| rewrite             | crates/jci-coverage/src/complexity/cobertura.rs:224 | 39                  | 71/75 (94%)  | 39.2          | complexity, CRAP (complexity) | reduce complexity below 15  |
| post_multipart      | crates/jci-coverage/src/upload/http.rs:135          | 5                   | 0/26 (0%)    | 30.0          | CRAP (coverage)               | cover 1 more line (1 of 26) |
```

*Over* says which limit a function breaks. A CRAP failure is marked *(coverage)* when more
tests would fix it, and *(complexity)* when they cannot, because CRAP is never below
complexity: a function whose complexity is at or over the CRAP limit has to be made simpler,
however well it is tested. *Fix* says what to do. For a coverage failure it gives how many of
the function's own lines must be covered to pass. A function over both limits needs both, and
says so: "reduce complexity below 15, then cover 15 more lines (15 of 26)", the second figure
being what it needs once it is simple enough. *Covered* is rounded down, so only a fully
covered function reads 100%.

With a baseline in use the table adds a *Baseline* column saying whether the function is new
or worse than it was recorded, and in which score ("complexity 31 > 29"). The report ends
with the ways to resolve a failure: reduce the complexity, add tests, accept the current state
with a baseline, or change the limits. The `--write-baseline` command it suggests repeats the
options in use (`--file`, `--source-root`, `--rules`, `--gate-tests`, the limits and the
baseline path), so it records the functions that failed. Every failing function is listed,
however many; a baseline is the way to adopt a limit on code that already breaks it.

`--summary-file <PATH>` writes a short markdown summary of the gate result: the headline
counts, a compact table (function, where it is over, fix) of the first 25 failing functions
with the rest counted, and a one-line pointer to the ways to resolve a failure and to the CI
log for the full report. There is no link to the job, since its log may not be readable by
everyone who can read the pull request. A passing gate gets a "passes" summary, with how
many functions over a limit the baseline allows. The file starts with a hidden marker,
`<!-- jci-coverage:gate -->`, so a later step can find the comment it belongs to and edit it
in place. Names and paths are put in code spans, with `|` escaped, and clipped, so a function
name cannot break the table. The file is written only when requested, when a maximum is set,
after the report, and not with `--dry-run` (it cannot be combined with `--write-baseline`); it may not be the report,
the baseline or any other output (compared by where the files are, not how they are
spelled). In CI it is ephemeral unless you persist it, for example as a build artifact; if
you keep it some other way, naming it, avoiding clashes and clearing stale copies are yours
to manage. A failure to write it is a warning and does not change the gate's result.

`--pr-comment` posts the same summary as a comment on the pull request the build is for
(`CIRCLE_PULL_REQUEST`), in the repository named by `CIRCLE_PROJECT_USERNAME` and
`CIRCLE_PROJECT_REPONAME`. The token is read from `GITHUB_TOKEN` (never a flag, so it stays
out of the command line and the logs) and the API address from `GITHUB_API_URL`, default
`https://api.github.com`; the token is only ever sent to `https`, or to `http` on this machine.
The comment is found again by its marker and edited in place, so a pull request carries one gate
comment however often CI runs. A failing gate creates it if there is none; a passing gate only
updates one that exists, so a pull request that never failed gets no comment. If GitHub refuses
the edit (the comment is not the token's to change, as when someone else has posted the marker),
the next-newest marked comment is tried; if none can be edited, a failing gate posts a new one. A
rate-limit refusal is an error, and so is a pull request with more than 5000 comments. Nothing is posted, and the reason is logged,
when there is no token (a warning), no pull request, or the pull request is from a fork (whose
build has no secrets). Like the summary file it needs a maximum, does nothing with `--dry-run` and cannot be
combined with `--write-baseline`, and a failure to post is a warning that does not change the gate's result.

A project adopting a maximum usually has functions that are already over it. Record them
with `--write-baseline` (which needs a maximum, writes nothing with `--dry-run`, and does not
fail), commit the file, and pass it back with `--baseline`. A baselined function then fails
only if it gets worse, with higher complexity or CRAP than recorded, and is still warned
about; a new function over the maximum fails. A function is identified by its file and name,
not its line, so functions of one name in one file (and closures) share an entry holding
their highest scores.

Without a value both options use `.jci-coverage-baseline` in the directory `jci-coverage`
runs from. By convention that is the repository root, committed beside the workspace
`Cargo.toml`: the file paths in the report, and so in the baseline, are relative to the
workspace root, where `complexity` already expects to run (`--source-root` defaults to `.`).
Give a path, as in `--baseline ci/complexity.bin`, to keep it elsewhere; a relative path is
resolved against the directory the command runs in, not the repository or package root.

The baseline is a small binary file for this tool, not meant to be read or edited: file
names are stored once, numbers are variable-length, and a checksum covers the whole file.
It is written atomically, and the same entries always give the same bytes, so it changes in
a commit only when something did. Git shows no text diff of it; add
`.jci-coverage-baseline binary` to `.gitattributes` to say so. A file that is damaged,
truncated, from a newer format or scored with the other `--rules` is refused rather than
trusted. The summary says how many entries are no longer needed once a function is fixed or
removed; regenerate the file to drop them. `--check-only` runs just these
checks and leaves the report as it is; it is what the orb job's last step uses.

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
| `report_complexity_and_upload` | **The default for Rust projects.** Generate coverage, add cyclomatic complexity to the Cobertura report and upload that, so OtterWise shows Complexity and CRAP as well as coverage. It writes only `coverage/cobertura.xml`, so a later job that needs `coverage/lcov.info` must set `format: both`. |
| `report_and_upload` | Rust projects that want the lcov report only: generate `coverage/lcov.info` and upload it in one job. No Complexity or CRAP. |
| `report` | Generate coverage only, e.g. to persist or store the report yourself. |
| `complexity` | Rust projects: add complexity to a Cobertura report an earlier job produced. |
| `upload` | Any project: upload a coverage file an earlier job produced (use `attach_workspace`). Needs no Rust toolchain. |

```yaml
orbs:
  jci-coverage: jerus-org/jci-coverage@0.0

workflows:
  validation:
    jobs:
      - jci-coverage/report_complexity_and_upload
```

`report_complexity_and_upload` runs `report`, `complexity` and then `upload` of
`coverage/cobertura.xml`, so the report path is fixed and there is no `file` parameter. It
writes only the Cobertura report, which is all it needs. It takes the same `package`,
`runner`, `nextest_profile`, `no_all_features`, `log_level`, `attach_workspace` and
`workspace_root` parameters as `report_and_upload`, and these more:

| Parameter | Default | Use it to |
|-----------|---------|-----------|
| `format` | `cobertura` | write `both` formats from the one test run, when a later job wants `coverage/lcov.info` too (it is not uploaded) |
| `unmatched` | `keep` | set `fail` so the job stops when a method cannot be matched to a source function, instead of leaving its complexity at 0 |
| `exclude_tests` | `false` | count `#[cfg(test)]` code and `#[test]` functions as 0 |
| `source_root` | `.` | read sources from another directory |
| `rules` | `rca` | count the stricter `mccabe` decision points |
| `warn_complexity` | `10` | warn about each function at or over this complexity (`0` is off) |
| `warn_crap` | `30` | warn about each function at or over this CRAP score (`0` is off) |
| `max_complexity` | `0` | fail the job, after the upload, if any function is at or over this complexity (`0` is off) |
| `max_crap` | `0` | fail the job, after the upload, if any function is at or over this CRAP score (`0` is off) |
| `baseline` | empty | path of a committed baseline, such as `.jci-coverage-baseline`, of the functions allowed to stay over a maximum |
| `gate_tests` | `false` | judge `#[cfg(test)]` code and `#[test]` functions against the thresholds too |

The maximums are checked by a last step, after `upload`, that runs `complexity --check-only`:
it leaves the report as it is, repeats nothing the annotating step already printed, and
fails the job only if a function is at or over a maximum and not allowed by the baseline.
Because it comes after the upload, a failing gate still updates the OtterWise dashboard
before the job goes red. With both maximums at their default of `0` it has nothing to check
and succeeds. To adopt a maximum on existing code, record today's functions once with
`jci-coverage complexity --max-complexity 15 --write-baseline`, commit
`.jci-coverage-baseline` at the repository root, and set `max_complexity: 15` and
`baseline: .jci-coverage-baseline`; after that only a new or worse function fails.

It is Rust-only. For a project in another language, or one that wants the lcov report only,
use `report_and_upload` or `upload`.

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
