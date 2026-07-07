# simplelocalize-download

A resilient, Rust-powered port of the [SimpleLocalize CLI](https://github.com/simplelocalize/simplelocalize-cli)
`download` command, shipped as a Python package with a [Typer](https://typer.tiangolo.com/) CLI.

**Scope by design:** the **`download`** command, **JSON only** (`single-language-json`). The core is
written in Rust (tokio + reqwest) for concurrent, retrying, atomic downloads and exposed to Python
via [PyO3](https://pyo3.rs/) / [maturin](https://www.maturin.rs/). No JVM required.

## Why

The Java CLI downloads files **sequentially with no retries** — a single transient hiccup aborts the
run. This port moves resilience into the tool: bounded concurrency, exponential backoff with jitter,
and atomic writes, so a flaky API doesn't fail your pipeline. It also drops the JVM: a
`uvx simplelocalize-download` one-liner replaces downloading a JAR, verifying its checksum, and
running `java -jar`.

## Install

```bash
uvx simplelocalize-download --help      # run without installing
pipx install simplelocalize-download    # or install as a tool
pip install simplelocalize-download     # or into an environment
```

## CLI usage

```bash
simplelocalize-download download \
  --api-key "$SIMPLELOCALIZE_API_KEY" \
  --download-path './json/{lang}/{ns}.json' \
  --download-options WRITE_NESTED \
  --download-sort LEXICOGRAPHICAL
```

The API key is read from `--api-key` or the `SIMPLELOCALIZE_API_KEY` environment variable.

### Options

| Option | Alias (Java CLI) | Notes |
| --- | --- | --- |
| `--api-key` | `--apiKey` | Or `SIMPLELOCALIZE_API_KEY`. Required. |
| `--download-path` | `--downloadPath` | Required. Template with `{lang}`, `{ns}`, `{customer}`, `{translationKey}`, `{remotePath}`. |
| `--download-format` | `--downloadFormat` | Default `single-language-json`; anything else is rejected. |
| `--download-options` | `--downloadOptions` | Comma-separated, e.g. `WRITE_NESTED`. |
| `--download-sort` | `--downloadSort` | e.g. `LEXICOGRAPHICAL`. |
| `--language-key` | `--downloadLanguageKey` | Comma-separated language filter. |
| `--namespace` | `--downloadNamespace` | Namespace filter. |
| `--tags` | `--downloadTags` | Comma-separated tag filter. |
| `--customer-id` | `--downloadCustomerId` | Customer ID filter. |
| `--base-url` | `--baseUrl` | Default `https://api.simplelocalize.io`. |
| `--concurrency` | — | Max concurrent file downloads (default 8). |
| `--timeout` | — | Per-request timeout in seconds. |
| `-v` / `-q` | — | Verbose (debug) / quiet (warnings only) logging. |
| `--no-progress` | — | Disable the progress bar. |

The Java CLI's camelCase flag spellings are accepted as aliases, so existing invocations port by
swapping only the binary name.

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success. |
| `2` | Usage / validation error (missing API key, unsupported format). |
| `3` | Authentication failed (HTTP 401/403). |
| `4` | API or network error (pre-flight). |
| `5` | Some files failed to download (partial failure). |

## Library usage

```python
import simplelocalize_download as sld

def on_event(event: dict) -> None:
    if event["type"] == "file_done":
        print("downloaded", event["path"])

try:
    report = sld.download(
        api_key="...",
        download_path="./json/{lang}/{ns}.json",
        download_options=["WRITE_NESTED"],
        download_sort="LEXICOGRAPHICAL",
        concurrency=8,
        on_event=on_event,
    )
    print(f"{len(report.downloaded)} files")
except sld.AuthError as err:
    print("bad credentials", err.status)
except sld.DownloadFailedError as err:
    for failed in err.report.failed:
        print("failed:", failed.url, failed.error)
```

Exceptions: `SimpleLocalizeError` (base) → `AuthError` (has `.status`), `ApiError` (has `.status`),
`NetworkError`, and `DownloadFailedError` (has `.report`).

## Resilience

- **Concurrency**: files download in parallel with a bounded limit (`--concurrency`, default 8).
- **Retries**: connect errors, timeouts, HTTP 429 and 5xx are retried with exponential backoff and
  full jitter, honoring `Retry-After`. Other 4xx fail fast.
- **Atomic writes**: each file streams to a temp file in the destination directory and is renamed
  into place, so a crash never leaves a half-written translation file.
- **Partial-failure reporting**: a failed file doesn't abort the batch; every file is attempted and
  failures are collected. The CLI exits non-zero (code 5) so CI still catches problems.

## What's not supported

This port intentionally covers only the `download` command for JSON. Not supported:

- Other commands: `upload`, `publish`, `auto-translate`, `pull`, `purge`, `extract`, `init`.
- Non-JSON formats (YAML, Android, iOS, Java properties, …) — every format other than
  `single-language-json`.
- `simplelocalize.yml` config files (pass everything via flags / environment).

For those, use the upstream [SimpleLocalize CLI](https://github.com/simplelocalize/simplelocalize-cli).

## Development

```bash
uv sync
uv run maturin develop
uv run pytest
```

See [CONTRIBUTING.md](CONTRIBUTING.md).
