# Pull Request Log

All notable pull requests merged into this workspace are recorded here. This log
tracks workspace-level changes (`v<VERSION>` tags); per-crate code changes are
tracked in each crate's `CHANGELOG.md` (`<crate>-v<VERSION>` tags).

## [Unreleased]

### Fixed

- deps: lock file maintenance(pr [#38])
- deps: update dependency jci-coverage to v0.0.10(pr [#37])
- upload: match OtterWise uploader's field formats(pr [#39])

## [0.0.10] - 2026-10-12

### Added

- detailed debug logging with the token withheld(pr [#36])

### Fixed

- deps: update dependency jci-coverage to v0.0.9(pr [#35])

## [0.0.9] - 2026-10-12

### Changed

- ci-dogfood the jci-coverage orb for coverage (P4)(pr [#33])

### Fixed

- upload: send absolute base_dir; log OtterWise response(pr [#34])

## [0.0.8] - 2026-10-11

### Fixed

- deps: lock file maintenance(pr [#28])
- deps: lock file maintenance(pr [#32])
- deps: update rust:1-slim-trixie docker digest to 2752b33(pr [#30])
- deps: update dependency gen-circleci-orb to v0.2.5(pr [#31])
- upload: tolerate missing parent blobs; short orb step names(pr [#29])

## [0.0.7] - 2026-10-04

### Changed

- ci-adopt the post-merge check and release gate(pr [#25])

### Fixed

- deps: update rust:1-slim-trixie docker digest to 70d3b1a(pr [#21])
- deps: update dependency toolkit to v8.0.2(pr [#22])
- deps: lock file maintenance(pr [#24])
- deps: update dependency toolkit to v8.0.3(pr [#26])
- deps: update dependency gen-circleci-orb to v0.2.4(pr [#23])
- deps: lock file maintenance(pr [#27])

## [0.0.6] - 2026-09-29

### Added

- default report_and_upload file to coverage/lcov.info(pr [#20])

### Fixed

- deps: update dependency gen-circleci-orb to v0.1.28(pr [#19])

## [0.0.5] - 2026-09-29

### Added

- generate the jci-coverage CircleCI orb (P3)(pr [#18])

### Changed

- docs-link roadmap P3/P4 to tracking issues and update P4 release reference(pr [#17])

## [0.0.4] - 2026-09-28

### Changed

- Make CLI tests resilient to host environment(pr [#14])

### Fixed

- deps: lock file maintenance(pr [#6])
- deps: lock file maintenance(pr [#7])
- deps: lock file maintenance(pr [#13])
- deps: update rust crate clap to 4.6.7(pr [#8])
- deps: update rust crate reqwest to 0.13.5(pr [#9])
- deps: update rust crate time to 0.3.55(pr [#10])
- deps: update dependency toolkit to v8(pr [#12])

## [0.0.3] - 2026-08-21

### Added

- implement upload command (P2)(pr [#5])

## [0.0.2] - 2026-08-20

### Added

- implement report command (P1)(pr [#4])

## [0.0.1] - 2026-08-19

### Added

- scaffold jci-coverage workspace (P0)(pr [#3])

[#3]: https://github.com/jerus-org/jci-coverage/pull/3
[#4]: https://github.com/jerus-org/jci-coverage/pull/4
[#5]: https://github.com/jerus-org/jci-coverage/pull/5
[#6]: https://github.com/jerus-org/jci-coverage/pull/6
[#7]: https://github.com/jerus-org/jci-coverage/pull/7
[#13]: https://github.com/jerus-org/jci-coverage/pull/13
[#8]: https://github.com/jerus-org/jci-coverage/pull/8
[#9]: https://github.com/jerus-org/jci-coverage/pull/9
[#10]: https://github.com/jerus-org/jci-coverage/pull/10
[#12]: https://github.com/jerus-org/jci-coverage/pull/12
[#14]: https://github.com/jerus-org/jci-coverage/pull/14
[#17]: https://github.com/jerus-org/jci-coverage/pull/17
[#18]: https://github.com/jerus-org/jci-coverage/pull/18
[#19]: https://github.com/jerus-org/jci-coverage/pull/19
[#20]: https://github.com/jerus-org/jci-coverage/pull/20
[#25]: https://github.com/jerus-org/jci-coverage/pull/25
[#21]: https://github.com/jerus-org/jci-coverage/pull/21
[#22]: https://github.com/jerus-org/jci-coverage/pull/22
[#24]: https://github.com/jerus-org/jci-coverage/pull/24
[#26]: https://github.com/jerus-org/jci-coverage/pull/26
[#23]: https://github.com/jerus-org/jci-coverage/pull/23
[#27]: https://github.com/jerus-org/jci-coverage/pull/27
[#28]: https://github.com/jerus-org/jci-coverage/pull/28
[#32]: https://github.com/jerus-org/jci-coverage/pull/32
[#30]: https://github.com/jerus-org/jci-coverage/pull/30
[#31]: https://github.com/jerus-org/jci-coverage/pull/31
[#29]: https://github.com/jerus-org/jci-coverage/pull/29
[#33]: https://github.com/jerus-org/jci-coverage/pull/33
[#34]: https://github.com/jerus-org/jci-coverage/pull/34
[#35]: https://github.com/jerus-org/jci-coverage/pull/35
[#36]: https://github.com/jerus-org/jci-coverage/pull/36
[#38]: https://github.com/jerus-org/jci-coverage/pull/38
[#37]: https://github.com/jerus-org/jci-coverage/pull/37
[#39]: https://github.com/jerus-org/jci-coverage/pull/39
[Unreleased]: https://github.com/jerus-org/jci-coverage/compare/v0.0.10...HEAD
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
