<!-- LTex: Enabled=false -->
# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.10] - 2026-10-12

Summary: Added[3], Fixed[1]

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

Summary: Chore[1], Fixed[3], Testing[1]

### Fixed

 - fix(deps): update rust crate time to 0.3.55
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

[Unreleased]: https://github.com/jerus-org/jci-coverage/compare/v0.0.9...HEAD
[0.0.9]: https://github.com/jerus-org/jci-coverage/compare/v0.0.8...v0.0.9
[0.0.8]: https://github.com/jerus-org/jci-coverage/compare/v0.0.7...v0.0.8
[0.0.7]: https://github.com/jerus-org/jci-coverage/compare/v0.0.6...v0.0.7
[0.0.6]: https://github.com/jerus-org/jci-coverage/compare/v0.0.5...v0.0.6
[0.0.5]: https://github.com/jerus-org/jci-coverage/compare/v0.0.4...v0.0.5
[0.0.4]: https://github.com/jerus-org/jci-coverage/compare/v0.0.3...v0.0.4
[0.0.3]: https://github.com/jerus-org/jci-coverage/compare/v0.0.2...v0.0.3
[0.0.2]: https://github.com/jerus-org/jci-coverage/compare/v0.0.1...v0.0.2
[0.0.1]: https://github.com/jerus-org/jci-coverage/releases/tag/v0.0.1

