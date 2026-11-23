# CLAUDE.md

Rules for Claude Code working in this repository.

## Overview

**jci-coverage** adds cyclomatic complexity and CRAP scores to a `cargo llvm-cov`
coverage report (Cobertura or lcov) and uploads it to OtterWise. It ships a crate
to crates.io and a generated CircleCI orb `jerus-org/jci-coverage` plus a
container image, in tag-lockstep, produced by `gen-circleci-orb`.

Two independent subcommands, and no subcommand that runs `cargo`: the project's
own job writes the report (`cargo llvm-cov --lcov`).

- `complexity` annotates the report, applies the gate (`--check-only`, maximums,
  baseline) and can post a pull request comment.
- `upload` posts a coverage file to OtterWise; it needs the CircleCI environment.

## Architecture

```
crates/jci-coverage/
├── src/
│   ├── main.rs             # mod declarations, tracing setup
│   ├── cli.rs              # Cli + Commands, dispatch
│   ├── complexity.rs       # the complexity subcommand
│   ├── complexity/         # cobertura, lcov, counter, plan, metrics, baseline, gate_report
│   ├── upload.rs           # the upload subcommand
│   ├── upload/             # ci_meta, git_meta, diff, diagnostics, otterwise, http
│   └── github.rs, github/  # the pull request comment
└── tests/cmd/*.trycmd      # CLI snapshot tests (trycmd)
orb/                        # GENERATED orb source and Dockerfile
gen-circleci-orb.toml       # the generator's config (hand-maintained)
.jci-coverage-baseline      # functions allowed to stay over the CRAP/complexity gate
```

- `orb/` is generated from the CLI's `--help` and `gen-circleci-orb.toml`: change
  the CLI or the config, then regenerate; never edit generated files by hand
  (`orb/src/examples/` is the exception, it is hand-written and listed under
  `custom_files`). The `# >>> gen-circleci-orb` regions of `.circleci/*.yml` are
  edited with `gen-circleci-orb update`, never by hand.
- `PRLOG.md` and the crate's `CHANGELOG.md` are generated from the Conventional
  Commit titles. Do not edit them.

## Development commands

```bash
just test            # clippy + check + doc + unit/CLI tests
just clippy          # cargo clippy --all --tests --all-features -- -D warnings
just audit           # jci-audit check (cargo-deny + cargo-audit)
just msrv            # cargo msrv verify (workstation tool)
just fmt             # nightly rustfmt + stable check
just cov             # lcov report with cargo llvm-cov, then complexity
just licenses        # regenerate THIRD-PARTY-LICENSES.md (cargo-about)
```

Regenerate CLI snapshots after intentional CLI changes:
`TRYCMD=overwrite cargo test --test cli_tests`, then read each diff. Use
`--dry-run` in snapshots that run `complexity` so nothing is written into the
`tests/cmd/*.in` directories.

## Git and pull requests

- Never commit to `main`. Branch from a freshly fetched `origin/main`, named `<type>/<description>` (`feat`, `fix`, `docs`, `chore`, `refactor`). A cloud session works on the branch it was given.
- Never merge, approve or close a pull request, including your own superseded ones: open it, return the URL, comment, and recommend closure.
- Stop and report before any commit, amend or push, and wait for "go", "commit and push" or "commit and pr". After a review or fixes, apply them locally, then stop.
- Stage explicit paths; never `git add -A`.
- Before pushing to a branch that has a PR, check the PR's state: it may have been merged. A `[new branch]` line on push means a merged branch was re-created.
- One PR at a time per workpackage: wait for the merge before starting the next unless told otherwise.
- Never force-push a Renovate branch. A fix needed only because of a Renovate update goes on that PR's branch as a new commit.
- Prefer merge commits to squash, to keep attribution.
- "Closes #N" in a commit or PR closes the issue on merge: use "Refs #N" unless the change finishes it.

## Commits

- Conventional Commits, subject under 50 characters, `git commit -s` for the sign-off. Commits are also GPG-signed as the agent's GitHub account, never as the generic `noreply@anthropic.com` identity: pcu's verifier cannot tie that address to a known operator and rejects it. Run `.claude/hooks/agent-signing.sh` at the start of a session and check `git var GIT_AUTHOR_IDENT` before the first commit; see `.claude/hooks/README.md` for the environment variables and the security rules for the dedicated environment.
- A commit made with Claude Code carries these trailers before the sign-off:

  ```
  Generated-by: claude-code/<version> (model: <model-id>; operator: <operator-email>)
  Claude-Session: <session-url>
  ```

  `<version>` is `claude --version`, `<model-id>` the model serving the session, `<operator-email>` the person directing it.
- Never write the literal CI-skip token in a commit or PR message.
- Tag prefixes: crate `jci-coverage-v<VERSION>`, workspace `v<VERSION>`. `release.toml` uses `consolidate-commits = false`.

## Rust work

- Red/green TDD for all Rust work, features, fixes and refactors: write the failing test, show it fail, then make it pass. Write characterisation tests before refactoring and mutation-check them (change the code, watch a test fail, revert).
- Put `#[cfg(test)]` modules at the end of the file with the existing ones. Tests that spawn git run with `GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1` and give fixtures their own identity.
- Edition 2024. `rust-version` (1.89) is the genuine minimum; do not contort code to protect a low MSRV. Verify with `just msrv` before every PR that changes dependencies or `Cargo.lock`; keep `rust-version` and `min_rust_version` in `.circleci/config.yml` in step.
- Before a PR, on every Rust PR even config-only ones, run what CI runs (read `.circleci/config.yml`): `just fmt` (nightly rustfmt via `rustfmt-nightly.toml`, then the stable check; revert drift onto unrelated files), `just clippy`, `just test`, `just audit`, and `just msrv` when dependencies or `Cargo.lock` changed. Re-run the format check after any edit. A new advisory can fail a PR that touches no Rust.
- CI also gates on: the CRAP gate (`complexity --check-only --max-complexity 15 --max-crap 30 --baseline .jci-coverage-baseline`: a function at or over 15 fails, so split rather than baseline), the nightly doc build with `RUSTDOCFLAGS="--cfg docsrs -Dwarnings" ... --document-private-items`, `gen-circleci-orb generate --check` and `update --check`, and SonarCloud new-code duplication at or under 3%.
- Run `/code-review` once per PR, on the final diff, before the commit. Report the findings and stop. Comment-only or docs-only PRs skip it.

## Working practice

- Fix the root cause, not the symptom. Age is not evidence of soundness.
- An advisory ignore in `deny.toml` needs a written justification, a link to the upstream tracking issue, and the condition for removing it. Prefer removing the dependency to ignoring the advisory. `deny.toml` is the single source of truth: `.cargo/audit.toml` and `crates/*/about.toml`'s accepted lists are derived from it by `jci-audit sync`, never edited by hand.
- File a follow-up as an issue as soon as it is found, labelled `investigate`, with the evidence, why it was deferred, the options and a recommendation. One item per issue. Security findings are the exception: see the next rule.
- Report a security finding (a way around a check, an exposed or over-broad credential, a gap in verification) privately, never in a public issue, PR text, comment, doc or commit message. Say nothing public about it, not even that one exists: a placeholder or "moved to a private report" note also draws attention. If unsure whether something is a security finding, treat it as one.
- Private routes, in order: the repository's private vulnerability reporting (Security tab, "Report a vulnerability"); when the session cannot reach it (the session proxy blocks writes to that API), write a report file (title, severity, description), send it to the user in the same turn with the file-delivery tool, and log it as an issue in the private findings repository `jerus-org/security-findings` (attach it with `add_repo`, push access), assigned to `jerusdp`. That repository is for trusted agents only; `SECURITY.md` never mentions it. Highlight the finding strongly in the chat until the user has filed it.
- Show a CI gate working, not just an explanation: reintroduce the failure, watch it fail, then revert.
- Comments state only the rule a reader cannot infer from the code, and describe current behaviour only. History belongs in commits, PRs and issues.
- Published text (PRs, issues, docs) uses placeholders, not real bot or user identities. Policy files that need real identifiers are the exception. Never print a token or key.
- Use the Context7 connector (`resolve-library-id`, `query-docs`) for library documentation when it is available.

## CI/CD

CircleCI, 3-file model (`.circleci/{config.yml,release.yml,update_prlog.yml}`) on `jerus-org/circleci-toolkit` (version pinned in those files, bumped by Renovate), plus `otterwise_drift.yml`. The orb pins `jerus-org/jci-coverage@<version>` are bumped by Renovate after each release. Release calculates versions before an approval gate, then `toolkit/release_crate` + `toolkit/release_prlog`; the `orb-release` workflow builds the image and publishes the orb. The coverage jobs after the prlog update in `update_prlog.yml` and `release.yml` carry a small `commands:` block to cover the head of `main`: no toolkit job can do that yet (digital-prstv/circleci-toolkit#564).
