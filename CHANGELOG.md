# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.2](https://github.com/anza-xyz/xtask/compare/anza-xtask@v0.2.1...anza-xtask@v0.2.2) - 2026-08-12

### Other

- add --no-verify to crate publish tests ([#39](https://github.com/anza-xyz/xtask/pull/39))

## [0.2.1](https://github.com/anza-xyz/xtask/compare/anza-xtask@v0.2.0...anza-xtask@v0.2.1) - 2026-08-11

### Fixed

- reset pre-release to -alpha.0 on minor and major bump ([#35](https://github.com/anza-xyz/xtask/pull/35))
- only reset local modifications expected to be done by test_update_crate ([#34](https://github.com/anza-xyz/xtask/pull/34))

### Other

- stop tests racing on the process working directory ([#36](https://github.com/anza-xyz/xtask/pull/36))

## [0.2.0](https://github.com/anza-xyz/xtask/compare/anza-xtask@v0.1.0...anza-xtask@v0.2.0) - 2026-07-30

### Added

- verify bump-version only edits version fields ([#31](https://github.com/anza-xyz/xtask/pull/31))

### Other

- extend update-crate command to update crates in dev-dependencies ([#30](https://github.com/anza-xyz/xtask/pull/30))
- rename bin ([#29](https://github.com/anza-xyz/xtask/pull/29))
- replace app-id with client-id ([#28](https://github.com/anza-xyz/xtask/pull/28))

## [0.1.0](https://github.com/anza-xyz/xtask/releases/tag/anza-xtask@v0.1.0) - 2026-05-12

### Other

- initial release
