# jci-coverage

Complexity and CRAP scores for
[`cargo-llvm-cov`](https://crates.io/crates/cargo-llvm-cov) coverage reports, and upload of
the report to [OtterWise](https://getotterwise.com) for code quality reporting.

[![Crates.io](https://img.shields.io/crates/v/jci-coverage.svg)](https://crates.io/crates/jci-coverage)
[![Documentation](https://docs.rs/jci-coverage/badge.svg)](https://docs.rs/jci-coverage)
[![License](https://img.shields.io/crates/l/jci-coverage.svg)](https://github.com/jerus-org/jci-coverage#license)
[![OtterWise Coverage](https://img.shields.io/endpoint?url=https://otterwise.app/badge/github/jerus-org/jci-coverage/coverage/9a4ad122-de69-4113-98ac-1358c62d545f)](https://otterwise.app/github/jerus-org/jci-coverage)

## Why

Two independent subcommands, not one:

- **`complexity`** — computes cyclomatic complexity for the Rust functions a Cobertura
  or lcov report covers and writes it into a Cobertura report. `cargo llvm-cov` leaves
  every complexity at zero, so without this step OtterWise shows Complexity and CRAP as 0. It
  reads the source files the report names and needs no other tool. Rust only: the
  complexity is counted from Rust source, so the report's files must be `.rs` (the coverage
  formats themselves are not tied to a language; see below).
- **`upload`** — takes a coverage file path and posts it to OtterWise. It never assumes
  `complexity` produced the file, so it works standalone in a non-Rust project against
  whatever coverage report that project's own build already generates. It must run in a
  CircleCI job: OtterWise accepts an upload from anywhere but shows only those that carry a
  CI identity (its own bash uploader does not work outside a CI either), so `upload` stops,
  before any network call, unless the environment has `CIRCLECI=true`, `CIRCLE_BUILD_NUM`,
  `CIRCLE_JOB`, `CIRCLE_PROJECT_USERNAME`, `CIRCLE_PROJECT_REPONAME` and (outside tag builds)
  `CIRCLE_BRANCH`, and the error names each one that is missing. A pull request and a user
  are not required, since a build need not have them. Setting the variables by hand gets
  past the check; whoever does takes responsibility for what OtterWise then shows.

Kept separate so the CLI surface can grow additional upload targets (Codecov,
Coveralls) later without reshaping complexity analysis.

The coverage report itself is written by your own job (`cargo llvm-cov`); there is no
subcommand that runs it. (A `report` subcommand did, up to 0.0.20: see
[Migrating from `report`](#migrating-from-report).)

> **Status:** early (0.0.x). `complexity` and `upload` are implemented, and the
> `jerus-org/jci-coverage` CircleCI orb is generated from them. The orb is
> published (from 0.0.5) — see [ROADMAP.md](../../ROADMAP.md).

## Runtime prerequisites

`complexity` and `upload` run no other tool: they read the coverage report and the Rust
source files, and `upload` needs a network connection and an OtterWise repository or
organisation token. The report comes from `cargo llvm-cov`, which your own job runs, with
the `llvm-tools-preview` rustup component:

```bash
cargo binstall cargo-llvm-cov
rustup component add llvm-tools-preview
```

## Installation

```bash
cargo binstall jci-coverage
# or
cargo install jci-coverage
```

## Usage

```bash
cargo llvm-cov --lcov --output-path coverage/lcov.info   # your own job writes the report
jci-coverage upload --file coverage/lcov.info            # uploads it to OtterWise
```

To report complexity and CRAP, annotate the report and upload that instead:

```bash
cargo llvm-cov --cobertura --output-path coverage/cobertura.xml
jci-coverage complexity                       # adds complexity to it, in place
jci-coverage upload --file coverage/cobertura.xml
```

`complexity` is Rust-only. It computes complexity from Rust source and matches it to the
functions `cargo llvm-cov` names in its Cobertura report, so it refuses (before reading any
source or writing anything) a report that lists a file that is not `.rs`, or that lacks
`cargo llvm-cov`'s per-function entries. A Cobertura report from another language or tool
can still be uploaded as it is with `upload`.

The input may also be the lcov report of `cargo llvm-cov --lcov`, which is smaller than the
Cobertura report. The format is told from the content of `--file`. An lcov report is
converted exactly as `cargo llvm-cov --cobertura` converts it (with the same
[`lcov2cobertura`](https://crates.io/crates/lcov2cobertura) crate), annotated, and written
as a Cobertura report: it is never replaced, so `--output` says where to write (by default
`coverage/cobertura.xml`) and is what `upload` takes next:

```bash
cargo llvm-cov --lcov --output-path coverage/lcov.info
jci-coverage complexity --file coverage/lcov.info    # writes coverage/cobertura.xml
jci-coverage upload --file coverage/cobertura.xml
```

An lcov report names its files by the absolute path they had where it was written, which is
often not where the sources are now. For each file `complexity` looks for the longest tail of
its path that exists under `--source-root`, and takes what is left of the path as the
directory to strip from all of them, so the report may come from another machine. A file
that does not share that directory (a dependency's source, say) is left as it is.
With `--check-only` the report is read and nothing is written, so the gate can read the lcov
report directly.

lcov is a standard format, but `complexity` needs more of it than the format requires. It
matches each function in the report to a function in the Rust source by name and start line,
so the report needs function records (`FN:` lines) whose names are Rust paths
(`crate::module::function`, as `cargo llvm-cov --lcov` writes them) and the files must be
`.rs`. A report without function records is refused. Only `cargo llvm-cov --lcov` has been
tried: lcov from another tool should work if it names the functions the same way, and is
refused or left unmatched if it does not.

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
comment however often CI runs. The comment is created if there is none, whether the gate passes or fails, so a pull request
always shows that the gate ran (a run that passes first time is not mistaken for one that did
not run). If GitHub refuses
the edit (the comment is not the token's to change, as when someone else has posted the marker),
the next-newest marked comment is tried; if none can be edited, a new one is posted. A
rate-limit refusal is an error, and so is a pull request with more than 5000 comments. Nothing is posted, and the reason is logged,
when there is no token (a warning), no pull request, or the pull request is from a fork (whose
build has no secrets). Like the summary file it needs a maximum, does nothing with `--dry-run` and cannot be
combined with `--write-baseline`, and a failure to post is a warning that does not change the gate's result.

A long failure list can be shortened and kept whole. `--max-listed N` lists at most `N` of the
failing functions, worst first, in the failure report in the CI log and in the summary (the
`--summary-file` and the pull request comment). The headline still counts every failing
function and the report says how many are not listed; 0, the default, lists them all. Whatever
it says, a pull request comment that would be over 60,000 characters (GitHub refuses one over
65,536) is cut to fit, and says how many it left out. `--failures-file PATH` writes every
failing function, not only the listed ones, as a markdown task list (one `- [ ]` item a line,
worst first, with where it is, why it fails and the fix), to copy from when tracking the
failures as issues in batches. The note about the functions not listed names the file. It is
written, after the report, only when the gate fails, never with `--dry-run`, and a failure to
write it is a warning. It cannot be the report, the summary or the baseline. The file is lost when a CI job ends unless a
step stores it. The generated `complexity_and_upload` job stores it: give `failures_file` a
path and the job ends with a `store_artifacts` step for it, which runs even though the gate
failed, so the file is among the job's artifacts. (Nothing is stored when the gate passes, as no
file is written; CircleCI notes that and the step succeeds.) `failures_file: "-"` puts the whole
list at the end of the failure report in the job log instead, headed "Every failing function
(N)", where it can be copied; with `--failures-file -` on the command, no file is written. In a
job of your own, run `complexity` with a `store_artifacts` step after it. The baselined functions that moved
and those that stay as recorded keep their own cap of 25 rows.

A project adopting a maximum usually has functions that are already over it. Record them
with `--write-baseline` (which needs a maximum, writes nothing with `--dry-run`, and does not
fail), commit the file, and pass it back with `--baseline`. A baselined function then fails
only if it gets worse in a score that is at or over its maximum: a function at or over
`--max-complexity` alone fails for higher complexity than recorded, one at or over `--max-crap`
alone for higher CRAP, and one at or over both for either. A score that is still under its
own maximum is not judged, so a CRAP that moves from 29.2 to 29.4 with the maximum at 30 does
not fail a function accepted for its complexity, while one that reaches 30 is at the maximum
and worse than recorded, and does. A baselined function is still warned about; a new function
at or over the maximum fails. A function is identified by its file and name, not its line, so
functions of one name in one file (and closures) share an entry holding their highest scores.

A baselined function whose scores have moved from the recorded ones is reported without failing
the gate, so a degradation that is still within what the baseline accepts is a prompt to rework
the function, and an improvement shows where the baseline can be recorded again to hold the
gain. The log warns about each function that got worse (any score worse than recorded, even
one under its maximum, such as a CRAP moving from 29.2 to 29.4) and notes each that got
better, up to ten; the `--summary-file` and the pull request comment gain a "Baselined
functions that moved" table, the degraded ones first. Every movement is reported, so coverage
that differs by a line between runs shows as a small CRAP change. A function that fails the
gate is reported as a failure and not again here.

The baselined functions that are as recorded are listed too, so it is clear what the baseline
lets through: the log names each (up to ten) after the baseline line, and the `--summary-file`
and the pull request comment gain a collapsed "baselined function(s) unchanged" table with the
scores, the one at or over its maximum in bold, worst first, capped like the other tables (the
heading has the full count). A function that moved is in its own table and not repeated there.

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
- An `async fn` is two methods in the report: the function and its generated future,
  `f::{closure#0}`, at the same line. The function keeps the complexity and the future is
  matched to it with none, so nothing is counted twice. An `async` block is a closure of
  its own, numbered with the closures around it. An `async` closure (`async |x| ..`) is the
  same: `cargo llvm-cov` (0.9.1; other versions may name these differently) reports it
  twice (the second time with no hits) and its future as `{closure#0}` below it, with a
  generic argument such as `::<i16>` on the end of the name.
  The closure keeps the complexity and the future adds none. With the default
  `--duplicates merge` the two entries of the closure become one; `--duplicates keep` leaves
  both, so an async closure then counts twice in OtterWise's sum, as a generic function
  counts once per instantiation.
- Sources are read relative to `--source-root` (default `.`), not the report's `<sources>`.
  Run from the directory the report was built in, or pass `--source-root`; if nothing
  matches, a warning says so.
- The report is replaced by a new regular file, so a symlink is not followed and the file's
  mode is not kept. Use `--output` to write the annotated report elsewhere.

See `jci-coverage complexity --help` and
`jci-coverage upload --help` for the full flag reference.

## CircleCI orb

The `jerus-org/jci-coverage` orb is generated from this CLI by
[`gen-circleci-orb`](https://github.com/jerus-org/gen-circleci-orb) (source in
[`orb/`](../../orb), config in [`gen-circleci-orb.toml`](../../gen-circleci-orb.toml)).
Its executor image is a slim Debian with `jci-coverage` and `git`: no Rust toolchain, no
`cargo`, and no `cargo-llvm-cov`. The jobs therefore take a report your own job wrote, in
your own executor; they never build your crate.

| Job | Use it for |
|-----|------------|
| `complexity_and_upload` | **The default for Rust projects.** Take the report your own job wrote with `cargo llvm-cov` (Cobertura or lcov) and persisted to the workspace, add cyclomatic complexity to it, upload it so OtterWise shows Complexity and CRAP as well as coverage, then apply the gate. See below. |
| `complexity` | Rust projects: add complexity to a Cobertura or lcov report an earlier job produced. |
| `upload` | Any project: upload a coverage file an earlier job produced (use `attach_workspace`). Needs no Rust toolchain. |

```yaml
orbs:
  jci-coverage: jerus-org/jci-coverage@0.0

workflows:
  validation:
    jobs:
      - coverage_report # your own job: cargo llvm-cov, then persist_to_workspace
      - jci-coverage/complexity_and_upload:
          requires: [coverage_report]
```

`complexity_and_upload` runs `complexity` and then `upload` of the annotated report, then
the gate (see "Writing the report in your own job" below). It takes `file` (the report, by
default `coverage/cobertura.xml`), `output`, `log_level`, `attach_workspace` and
`workspace_root`, and these more:

| Parameter | Default | Use it to |
|-----------|---------|-----------|
| `unmatched` | `keep` | set `fail` so the job stops when a method cannot be matched to a source function, instead of leaving its complexity at 0 |
| `exclude_tests` | `false` | count `#[cfg(test)]` code and `#[test]` functions as 0 |
| `source_root` | `.` | read sources from another directory |
| `rules` | `rca` | count the stricter `mccabe` decision points |
| `warn_complexity` | `10` | warn about each function at or over this complexity (`0` is off) |
| `warn_crap` | `30` | warn about each function at or over this CRAP score (`0` is off) |
| `max_complexity` | `0` | fail the job, after the upload, if any function is at or over this complexity (`0` is off) |
| `max_crap` | `0` | fail the job, after the upload, if any function is at or over this CRAP score (`0` is off) |
| `baseline` | empty | path of a committed baseline, such as `.jci-coverage-baseline`, of the functions allowed to stay over a maximum |
| `pr_comment` | `false` | post the gate's result as one comment on the pull request, edited in place on each run (see below) |
| `max_listed` | `0` | list at most this many failing functions in the failure report and the pull request comment; 0 lists them all |
| `failures_file` | empty | where to write every failing function, as a markdown task list; the job keeps it as an artifact when the gate fails. `-` puts the list at the end of the failure report in the job log instead; empty adds nothing |
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

`pr_comment: true` passes `--pr-comment` to that last step, so the gate's result is also
posted as one comment on the pull request, whether the gate passes or fails, and edited in
place on each run. The gate is the job's last step, after the upload, so a job that fails
earlier (the report, the upload) posts nothing and leaves any earlier comment as
it was. It needs a maximum to have a result to post, and a GitHub token with permission to
comment on pull requests in the `GITHUB_TOKEN` environment variable. Take the token from a
[context](https://circleci.com/docs/contexts/) or the project's environment variables, never
from a parameter, which would be stored in your config. Nothing is posted, and the reason is
logged, for a build with no pull request or a pull request from a fork (CircleCI gives a
fork's build neither a context nor your project's variables), and a failure to post is a
warning that does not change the gate's result. Add the context to the job in the workflow:

```yaml
      - jci-coverage/complexity_and_upload:
          requires: [coverage_report]
          context: my-github-token-context
          max_complexity: 15
          baseline: .jci-coverage-baseline
          pr_comment: true
```

The `pr_comment` parameter exists only in the orb release that adds it, so pin an orb version
at or after that one to use it.

It is Rust-only. For a project in another language, use `upload`.

Set the OtterWise token as the `OTTERWISE_TOKEN` environment variable in each
project's settings. The `upload` job's `repo_token` parameter exists
because it mirrors the CLI flag, but a token passed that way is stored in your
config and shown on the command line, so don't use it. See
[`orb/src/examples`](../../orb/src/examples) for a non-Rust, upload-only workflow.

### Writing the report in your own job

Making the coverage report compiles and runs your tests under instrumentation, so it needs
your toolchain and native libraries (a workspace that links openssl needs `pkg-config` and
`libssl-dev`, for example). The orb's image carries none of them, and its jobs take no
`executor` parameter, so the report is written by a job of your own, in your own executor
(the circleci-toolkit's `code_coverage` job is one), and persisted. `complexity` and
`upload` compile nothing: they need only the report and the source files.

```yaml
jobs:
  coverage_report:
    docker:
      - image: cimg/rust:1.91 # your toolchain, native libraries and cargo-llvm-cov
    steps:
      - checkout
      - run: cargo llvm-cov --lcov --output-path coverage/lcov.info
      - persist_to_workspace:
          root: .
          paths: [coverage/lcov.info]

workflows:
  validation:
    jobs:
      - coverage_report
      - jci-coverage/complexity_and_upload:
          requires: [coverage_report]
          file: coverage/lcov.info
```

`complexity_and_upload` checks out the source (the report names its files relative to the
repository root), attaches the workspace (`attach_workspace` is `true` here, and
`workspace_root` is where it is attached), annotates the report, uploads it and then checks
the gate, so a failing gate still updates the dashboard. `file` names the report relative to
the workspace (`coverage/cobertura.xml` by default), and `output` where the annotated
Cobertura report goes, which is what is uploaded and checked (also `coverage/cobertura.xml`,
so a Cobertura report is annotated in place). An lcov report (`cargo llvm-cov --lcov
--output-path coverage/lcov.info`) is converted: set `file: coverage/lcov.info` and leave
`output`; a Cobertura report (`cargo llvm-cov --cobertura --output-path
coverage/cobertura.xml`) needs neither.

### Migrating from `report`

The `report` subcommand, and the orb jobs `report`, `report_and_upload` and
`report_complexity_and_upload` that ran it, were removed after 0.0.20, and the orb's image
no longer carries a Rust toolchain, `cargo-llvm-cov` or `cargo-nextest`. To migrate:

1. Write the report in a job of your own, as above (`cargo llvm-cov --lcov --output-path
   coverage/lcov.info`, or with `cargo nextest`: `cargo llvm-cov nextest --lcov ...`), and
   persist it to the workspace.
2. Replace `report_complexity_and_upload` with `complexity_and_upload`, `report_and_upload`
   with `upload`, and `report` with your own job. The parameters that only `report` used
   (`package`, `runner`, `nextest_profile`, `no_all_features`, `format`) are gone: pass the
   equivalents to `cargo llvm-cov` in your job.
3. To stay on the old jobs for now, pin the orb to `jerus-org/jci-coverage@0.0.20`. A
   floating `@0.0` takes the removal.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
