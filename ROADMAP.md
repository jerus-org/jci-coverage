<!--
SPDX-FileCopyrightText: 2026 jerusdp

SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Roadmap

_Last updated: 2026-08-20._

This roadmap describes the intended direction of jci-coverage over roughly the next
year. It is a statement of intent, not a commitment: priorities may shift with user
feedback and maintainer availability (see [GOVERNANCE.md](GOVERNANCE.md)). Concrete
work is tracked in the [issue tracker](https://github.com/jerus-org/jci-coverage/issues);
this document groups that work into themes and horizons.

## Current status

jci-coverage is **pre-pre-release (0.0.x)**. The workspace and release machinery are
scaffolded and `report` is implemented; `upload` is still stubbed.

## Phased plan to 0.1.0 (preview)

| Phase | Scope | Status |
|-------|-------|--------|
| **P0 — scaffold** | Workspace, clap skeleton (flags settled, behaviour stubbed), release machinery, `jci-audit`-managed `deny.toml`/license policy | Done |
| **P1 — `report`** | Orchestrate `cargo-llvm-cov` (test + nextest runners), write `coverage/lcov.info`, terminal summary | Done |
| **P2 — `upload`** | Standalone multipart upload to OtterWise (repo/org token, git metadata, diff-coverage payload) | Planned |
| **P3 — generated orb** | `gen-circleci-orb`-produced `jerus-org/jci-coverage` orb; example workflows for a Rust repo and an upload-only non-Rust repo | Planned |
| **P4 — dogfooding + first releases** | jci-coverage's own CI runs `report`/`upload`; `jci-coverage-v0.0.1` validates the full loop | Planned |

## Near term (before 1.0 preview / `0.1.0`)

- **Project hardening / OpenSSF Best Practices badge.** Complete the governance,
  security, and quality documentation and achieve (and display) at least the Silver
  badge, reusing the playbook already validated on `gen-circleci-orb` and `jci-audit`.
- **Documentation and a project presence** (user guides, jrussell.ie project page,
  announcement draft).

## Backlog (deliberately out of MVP scope, tracked here rather than built ahead of need)

- **Wire `jci-audit check`/`sync --check` into CI once its orb is consumable.**
  `jci-audit` is available as a workstation tool (`just audit`, `just` uses it locally)
  but is deliberately **not** installed ad-hoc into a shared toolkit executor in CI
  (tried this, reverted it — `cargo binstall`/`cargo install`-ing it per job is exactly
  the kind of workaround the project is avoiding elsewhere; `jci-audit` is meant to be
  consumed via its own generated orb, whose self-contained executor image already
  bundles the binary with `cargo-audit`/`cargo-deny`/`cargo-about`). Its own roadmap
  defers consumer migration until it reaches 0.1.0. Until then, `deny.toml`/
  `about.toml` drift is a local-only check (`just audit`), not CI-enforced.
- **Adopt `jci-audit check`'s cargo-about resolvability check once it lands
  ([jerus-org/jci-audit#80](https://github.com/jerus-org/jci-audit/issues/80)).**
  `jci-audit check`/`sync --check` only verify that a crate's `about.toml` matches
  `deny.toml`'s policy — they don't verify `cargo-about` can actually *resolve* every
  dependency's licence (a distinct, real failure mode; `jci-audit release` already
  checks this at release time, per `crates/jci-audit/src/release.rs`). jci-audit's own
  repo currently plugs this PR-time gap with a hand-rolled script
  (`scripts/licenses.sh --policy`) that #80 proposes retiring once the check moves
  into the CLI. Deliberately not building that same workaround here: both projects
  are being developed together, and it's expected to land before jci-coverage reaches
  0.1.0. Accepted interim gap: an unresolvable licence is caught at release time
  (`crates/jci-coverage/release-hook.sh`'s `cargo about generate`), not at PR time.
- **Other upload targets** (Codecov, Coveralls) — `upload`'s internal target
  abstraction is designed for this, but only OtterWise is implemented for 0.1.0.
- **OtterWise upload-splitting** (`--part`/`--part-total`) for very large coverage
  reports — OtterWise's reference uploader supports it; not needed at current scale.
- **Mutation coverage / type coverage uploads** — OtterWise supports these for other
  language ecosystems (notably PHP); not applicable to a Rust-focused tool today.
- **JUnit log/config file uploads** — same reasoning; revisit if OtterWise's Rust
  support grows to use them.

## Medium term — toward 1.0

- **Stabilise the CLI surface.** Settle `report`/`upload`'s flags so that `0.x → 1.0`
  is a stability milestone with documented migration guidance for existing consumers.
- **Consumer migration.** Offer the orb as a replacement for
  `circleci-toolkit`'s `code_coverage` job across the org's Rust repos, once the MVP
  parity gap (if any remains — see the crate README's status note) is closed or
  explicitly accepted.

## Longer term (beyond 1.0)

- Fold `jci-coverage` into consumers' shared CI job sets as the default coverage
  gate, once the CLI surface is stable and consumer migration is complete.
