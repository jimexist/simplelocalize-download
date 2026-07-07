# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0]

Initial release: a resilient Rust port of the SimpleLocalize CLI `download` command with Python
bindings and a Typer CLI.

### Added

- Rust core (tokio + reqwest/rustls): SimpleLocalize `/cli/v2/download` API client and a concurrent,
  retrying download engine with atomic writes and path-template rendering.
- Python bindings (PyO3): synchronous `download()` that releases the GIL, a `DownloadReport` result
  type, an exception hierarchy (`SimpleLocalizeError`, `AuthError`, `ApiError`, `NetworkError`,
  `DownloadFailedError`), an optional progress callback, and Rust-to-Python log forwarding.
- Typer CLI `simplelocalize-download download` with loguru logging, a rich progress bar, distinct
  exit codes, and the Java CLI's camelCase flag spellings as aliases.

### Scope

- Supports the `download` command with the `single-language-json` format only. See the README for
  the full list of what is intentionally out of scope.

[Unreleased]: https://github.com/jimexist/simplelocalize-download/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jimexist/simplelocalize-download/releases/tag/v0.1.0
