# Pull Request Log

All notable pull requests merged into this workspace are recorded here. This log
tracks workspace-level changes (`v<VERSION>` tags); per-crate code changes are
tracked in each crate's `CHANGELOG.md` (`<crate>-v<VERSION>` tags).

## [Unreleased]

### Changed

- ci-cover the head of main from the prlog job(pr [#89])
- refactor(complexity)-split the Cobertura parse(pr [#92])
- ci-record the baseline again after the parse split(pr [#93])
- refactor(complexity)-split the complexity run(pr [#94])
- refactor(upload)-test and split the HTTP post(pr [#95])
- refactor(complexity)-split the baseline decode(pr [#96])

### Fixed

- deps: update dependency jci-coverage to v0.0.18(pr [#88])

## [0.0.18] - 2026-10-30

### Added

- list the baselined functions(pr [#87])

### Changed

- refactor(upload)-split summarize_cobertura(pr [#85])
- ci-record the baseline again after the split(pr [#86])

### Fixed

- deps: update dependency jci-coverage to v0.0.17(pr [#84])

## [0.0.17] - 2026-10-30

### Added

- report baselined functions that moved(pr [#83])

### Fixed

- deps: update dependency jci-coverage to v0.0.16(pr [#81])
- complexity: judge a baselined function on the limit it is over(pr [#82])

## [0.0.16] - 2026-10-30

### Changed

- ci-comment the coverage gate result on pull requests(pr [#74])
- ci-gate the dogfood job on risky functions(pr [#64])
- ci-upload coverage for the head of main after the prlog(pr [#76])
- docs-add the OtterWise coverage badge(pr [#77])
- ci-upload coverage for the head of main after a release(pr [#78])

### Fixed

- deps: update rust:1-slim-trixie docker digest to b8e062b(pr [#72])
- deps: update dependency jci-coverage to v0.0.15(pr [#73])
- deps: update libz-sys to 1.1.31(pr [#79])

## [0.0.15] - 2026-10-28

### Added

- write a markdown summary of the gate result(pr [#69])
- post the gate summary as a PR comment(pr [#70])
- add pr_comment to report_complexity_and_upload(pr [#71])

### Fixed

- deps: update dependency jci-coverage to v0.0.14(pr [#67])

## [0.0.14] - 2026-10-27

### Added

- report gate failures as a table with fixes(pr [#66])

### Fixed

- deps: lock file maintenance(pr [#62])
- deps: update rust:1-slim-trixie docker digest to 1a93974(pr [#61])
- deps: update dependency jci-coverage to v0.0.13(pr [#63])

## [0.0.13] - 2026-10-26

### Added

- make report_complexity_and_upload the default(pr [#51])
- warn on risky functions(pr [#57])
- fail on --max-complexity and --max-crap(pr [#58])
- add --baseline and --write-baseline(pr [#59])
- gate the composed job after the upload(pr [#60])

### Changed

- ci-dogfood report_complexity_and_upload(pr [#50])
- ci-address every option in the composed job(pr [#56])

### Fixed

- deps: update rust:1-slim-trixie docker digest to 4df2ae8(pr [#48])
- deps: update dependency jci-coverage to v0.0.12(pr [#49])
- deps: lock file maintenance(pr [#55])
- deps: update rust:1-slim-trixie docker digest to aab4239(pr [#53])
- deps: update dependency gen-circleci-orb to v0.2.6(pr [#54])

## [0.0.12] - 2026-10-18

### Added

- add --format lcov|cobertura|both(pr [#45])
- add complexity subcommand(pr [#46])
- add report_complexity_and_upload orb job(pr [#47])

### Fixed

- deps: update dependency jci-coverage to v0.0.11(pr [#41])

## [0.0.11] - 2026-10-15

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
[#41]: https://github.com/jerus-org/jci-coverage/pull/41
[#45]: https://github.com/jerus-org/jci-coverage/pull/45
[#46]: https://github.com/jerus-org/jci-coverage/pull/46
[#47]: https://github.com/jerus-org/jci-coverage/pull/47
[#48]: https://github.com/jerus-org/jci-coverage/pull/48
[#49]: https://github.com/jerus-org/jci-coverage/pull/49
[#50]: https://github.com/jerus-org/jci-coverage/pull/50
[#51]: https://github.com/jerus-org/jci-coverage/pull/51
[#55]: https://github.com/jerus-org/jci-coverage/pull/55
[#53]: https://github.com/jerus-org/jci-coverage/pull/53
[#56]: https://github.com/jerus-org/jci-coverage/pull/56
[#54]: https://github.com/jerus-org/jci-coverage/pull/54
[#57]: https://github.com/jerus-org/jci-coverage/pull/57
[#58]: https://github.com/jerus-org/jci-coverage/pull/58
[#59]: https://github.com/jerus-org/jci-coverage/pull/59
[#60]: https://github.com/jerus-org/jci-coverage/pull/60
[#62]: https://github.com/jerus-org/jci-coverage/pull/62
[#61]: https://github.com/jerus-org/jci-coverage/pull/61
[#63]: https://github.com/jerus-org/jci-coverage/pull/63
[#66]: https://github.com/jerus-org/jci-coverage/pull/66
[#67]: https://github.com/jerus-org/jci-coverage/pull/67
[#69]: https://github.com/jerus-org/jci-coverage/pull/69
[#70]: https://github.com/jerus-org/jci-coverage/pull/70
[#71]: https://github.com/jerus-org/jci-coverage/pull/71
[#72]: https://github.com/jerus-org/jci-coverage/pull/72
[#73]: https://github.com/jerus-org/jci-coverage/pull/73
[#74]: https://github.com/jerus-org/jci-coverage/pull/74
[#64]: https://github.com/jerus-org/jci-coverage/pull/64
[#76]: https://github.com/jerus-org/jci-coverage/pull/76
[#77]: https://github.com/jerus-org/jci-coverage/pull/77
[#79]: https://github.com/jerus-org/jci-coverage/pull/79
[#78]: https://github.com/jerus-org/jci-coverage/pull/78
[#81]: https://github.com/jerus-org/jci-coverage/pull/81
[#82]: https://github.com/jerus-org/jci-coverage/pull/82
[#83]: https://github.com/jerus-org/jci-coverage/pull/83
[#84]: https://github.com/jerus-org/jci-coverage/pull/84
[#85]: https://github.com/jerus-org/jci-coverage/pull/85
[#86]: https://github.com/jerus-org/jci-coverage/pull/86
[#87]: https://github.com/jerus-org/jci-coverage/pull/87
[#88]: https://github.com/jerus-org/jci-coverage/pull/88
[#89]: https://github.com/jerus-org/jci-coverage/pull/89
[#92]: https://github.com/jerus-org/jci-coverage/pull/92
[#93]: https://github.com/jerus-org/jci-coverage/pull/93
[#94]: https://github.com/jerus-org/jci-coverage/pull/94
[#95]: https://github.com/jerus-org/jci-coverage/pull/95
[#96]: https://github.com/jerus-org/jci-coverage/pull/96
[Unreleased]: https://github.com/jerus-org/jci-coverage/compare/v0.0.18...HEAD
[0.0.18]: https://github.com/jerus-org/jci-coverage/compare/v0.0.17...v0.0.18
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
