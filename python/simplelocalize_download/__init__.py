"""Resilient port of the SimpleLocalize CLI ``download`` command.

The heavy lifting (HTTP, concurrency, retries) lives in the compiled Rust
extension :mod:`simplelocalize_download._core`. This package re-exports the
public surface; the Typer CLI lives in :mod:`simplelocalize_download.cli`.
"""

from __future__ import annotations

from ._core import (
    ApiError,
    AuthError,
    DownloadedFile,
    DownloadFailedError,
    DownloadReport,
    FailedFile,
    NetworkError,
    SimpleLocalizeError,
    __version__,
    download,
)

__all__ = [
    "ApiError",
    "AuthError",
    "DownloadFailedError",
    "DownloadReport",
    "DownloadedFile",
    "FailedFile",
    "NetworkError",
    "SimpleLocalizeError",
    "__version__",
    "download",
]
