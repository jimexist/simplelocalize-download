# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- A server-supplied `Retry-After` no longer overrides the attempt budget. `backon` signals "budget
  exhausted" by passing no delay to the backoff adjuster, and the adjuster returned the server's hint
  regardless — so any server answering a 429/5xx with `Retry-After` retried **forever**, ignoring
  `--retry-attempts` and hanging unattended runs. `Retry-After` is now also clamped to the delay
  ceiling, so a server cannot decide how long a CI job sleeps.

### Changed

- Raise the default retry budget from 5 attempts / 250ms base / 10s ceiling to 8 attempts / 1s base /
  30s ceiling. The old defaults spent only ~3.75s of total backoff, so a SimpleLocalize 503 lasting
  more than a few seconds exhausted the retries and failed the run. Note this budget is per
  operation: each file download pays it as well as the list call, so a persistent failure over a
  large batch costs roughly `(files / concurrency) x budget`.
- Log each retry at `warn` instead of `debug`, and log a final `giving up after N attempt(s)` line on
  both the list call and each file download. Previously an unattended run showed only the terminal
  error, which read as if no retry had happened.
- An invalid `--timeout` / `--retry-*` value now exits with the usage code instead of a traceback.

### Added

- `--retry-attempts`, `--retry-base-delay` and `--retry-max-delay` (with `SIMPLELOCALIZE_RETRY_*`
  environment variable equivalents) and the corresponding `retry_attempts` /
  `retry_base_delay_secs` / `retry_max_delay_secs` arguments to `sld.download()`. The retry policy
  was previously hardcoded with no way to tune it from Python or the CLI.
- Retry logging for per-file downloads, which had no retry notification hook at all.

## [0.1.1]

### Changed

- Require Python 3.11 or newer; Python 3.10 is no longer supported.

### Fixed

- Derive `SPLIT_BY_LANGUAGES` / `SPLIT_BY_NAMESPACES` / `SPLIT_BY_CUSTOMERS` download options from
  the `{lang}` / `{ns}` / `{customer}` placeholders in the download path, matching the official CLI.
  Without them the API returns a single bundled file with null metadata, so the placeholders rendered
  empty (e.g. `json/.json`).

### Internal

- CI: run `ruff` in a dedicated fast lint job, cancel outdated concurrent runs, and fix `rustfmt`
  formatting.
- CI and dev env: align Python pins to 3.14.

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

[Unreleased]: https://github.com/jimexist/simplelocalize-download/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/jimexist/simplelocalize-download/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jimexist/simplelocalize-download/releases/tag/v0.1.0
