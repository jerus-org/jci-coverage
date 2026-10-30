<!-- LTex: Enabled=false -->
# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.18] - 2026-10-30

Summary: Added[1], Changed[1], Fixed[1], Testing[1]

### Added

 - feat(complexity): list the baselined functions

### Fixed

 - fix(complexity): drop the false pointer from the moved table

### Changed

 - refactor(upload): split summarize_cobertura

## [0.0.17] - 2026-10-30

Summary: Added[1], Chore[1], Fixed[2], Testing[2]

### Added

 - feat(complexity): report baselined functions that moved

### Fixed

 - fix(complexity): tighten the moved report after review
 - fix(complexity): judge a baselined function on the limit it is over

## [0.0.16] - 2026-10-30

Summary: Chore[1], Documentation[1]

## [0.0.15] - 2026-10-28

Summary: Added[3], Changed[1], Chore[1], Documentation[1], Fixed[3], Testing[1]

### Added

 - feat(orb): add pr_comment to report_complexity_and_upload
 - feat(complexity): post the gate summary as a PR comment
 - feat(complexity): write a markdown summary of the gate result

### Fixed

 - fix(github): post the summary when the gate passes too
 - fix(github): harden comment finding, exclude --write-baseline
 - fix(complexity): say a baseline is an alternative to fixing

### Changed

 - refactor(complexity): drop job link, tighten summary

## [0.0.14] - 2026-10-27

Summary: Added[1], Chore[1], Fixed[1]

### Added

 - feat(complexity): report gate failures as a table with fixes

### Fixed

 - fix(complexity): address review of the gate report

## [0.0.13] - 2026-10-26

Summary: Added[5], Changed[1], Chore[1], Documentation[3], Fixed[6], Testing[2]

### Added

 - feat(orb): gate the composed job after the upload
 - feat(complexity): default the baseline to .jci-coverage-baseline
 - feat(complexity): add --baseline and --write-baseline
 - feat(complexity): fail on --max-complexity and --max-crap
 - feat(complexity): warn on risky functions

### Fixed

 - fix(complexity): address review of --check-only
 - fix(complexity): address review of the baseline
 - fix(complexity): report a failing function once
 - fix(complexity): address review of risky warnings
 - fix: resolve Cobertura paths against <source>
 - fix: summarise Cobertura paths in the upload log

### Changed

 - refactor(complexity): store the baseline as a checked binary file

## [0.0.12] - 2026-10-18

Summary: Added[5], Chore[1], Documentation[2], Fixed[2]

### Added

 - feat: expose complexity options on composed job
 - feat: add report_complexity_and_upload orb job
 - feat: require a Rust cargo-llvm-cov report
 - feat: add complexity subcommand
 - feat(report): add --format lcov|cobertura|both

### Fixed

 - fix: stop help prose generating a bogus flag
 - fix: harden complexity matching and parsing

## [0.0.11] - 2026-10-15

Summary: Chore[1], Fixed[3], Testing[1]

### Fixed

 - fix(deps): update rustls to 0.23.46
 - fix(upload): address review of the field alignment
 - fix(upload): match OtterWise uploader's field formats

## [0.0.10] - 2026-10-12

Summary: Added[3], Chore[1], Fixed[1]

### Added

 - feat(upload): show the absolute report path
 - feat(upload): log the attached report filename
 - feat(upload): detailed debug logging, token withheld

### Fixed

 - fix(upload): strip remote credentials from logs

## [0.0.9] - 2026-10-12

Summary: Chore[1], Fixed[1]

### Fixed

 - fix(upload): send absolute base_dir; log response

## [0.0.8] - 2026-10-11

Summary: Added[1], Chore[1], Continuous Integration[2], Documentation[1], Fixed[2]

### Added

 - feat(upload)!: drop OTTERWISE_ORG_TOKEN and --org-token

### Fixed

 - fix(orb): short step names; drop token context
 - fix(upload): tolerate missing parent blobs

## [0.0.7] - 2026-10-04

Summary: Chore[1]

## [0.0.6] - 2026-09-29

Summary: Added[1], Chore[1]

### Added

 - feat: default report_and_upload file to coverage/lcov.info

## [0.0.5] - 2026-09-29

Summary: Added[1], Chore[1], Fixed[1]

### Added

 - feat: generate the jci-coverage CircleCI orb (P3)

### Fixed

 - fix: keep enum types on report_and_upload params

## [0.0.4] - 2026-09-28

Summary: Chore[1], Fixed[2], Testing[1]

### Fixed

 - fix(deps): update rust crate reqwest to 0.13.5
 - fix(deps): update rust crate clap to 4.6.7

## [0.0.3] - 2026-08-21

Summary: Added[1], Chore[1], Testing[1]

### Added

 - feat: implement upload command (P2)

## [0.0.2] - 2026-08-20

Summary: Added[1], Chore[1], Fixed[1]

### Added

 - feat: implement report command (P1)

### Fixed

 - fix: address PR review comments

## [0.0.1] - 2026-08-19

Summary: Added[1], Chore[1]

### Added

 - feat: scaffold jci-coverage workspace (P0)

[Unreleased]: https://github.com/jerus-org/jci-coverage/compare/v0.0.17...HEAD
[0.0.17]: https://github.com/jerus-org/jci-coverage/compare/v0.0.16...v0.0.17
[0.0.16]: https://github.com/jerus-org/jci-coverage/compare/v0.0.15...v0.0.16
[0.0.15]: https://github.com/jerus-org/jci-coverage/compare/v0.0.14...v0.0.15
[0.0.14]: https://github.com/jerus-org/jci-coverage/compare/v0.0.13...v0.0.14
[0.0.13]: https://github.com/jerus-org/jci-coverage/compare/v0.0.12...v0.0.13
[0.0.12]: https://github.com/jerus-org/jci-coverage/compare/v0.0.11...v0.0.12
[0.0.11]: https://github.com/jerus-org/jci-coverage/compare/v0.0.10...v0.0.11
[0.0.10]: https://github.com/jerus-org/jci-coverage/compare/v0.0.9...v0.0.10
[0.0.9]: https://github.com/jerus-org/jci-coverage/compare/v0.0.8...v0.0.9
[0.0.8]: https://github.com/jerus-org/jci-coverage/compare/v0.0.7...v0.0.8
[0.0.7]: https://github.com/jerus-org/jci-coverage/compare/v0.0.6...v0.0.7
[0.0.6]: https://github.com/jerus-org/jci-coverage/compare/v0.0.5...v0.0.6
[0.0.5]: https://github.com/jerus-org/jci-coverage/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/jerus-org/jci-coverage/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/jerus-org/jci-coverage/compare/v0.0.2...v0.0.3
[0.0.2]: https://github.com/jerus-org/jci-coverage/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/jerus-org/jci-coverage/releases/tag/v0.0.1

