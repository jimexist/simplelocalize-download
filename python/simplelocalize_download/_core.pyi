"""Type stubs for the compiled ``simplelocalize_download._core`` extension.

Kept in sync by hand with ``src/python.rs``.
"""

from collections.abc import Callable
from typing import Any

__version__: str

class DownloadedFile:
    """A file that downloaded successfully."""

    path: str
    language: str | None
    namespace: str | None
    bytes: int

class FailedFile:
    """A file that failed to download."""

    url: str
    language: str | None
    namespace: str | None
    error: str

class DownloadReport:
    """The outcome of a download batch."""

    downloaded: list[DownloadedFile]
    failed: list[FailedFile]
    def is_success(self) -> bool: ...

class SimpleLocalizeError(Exception):
    """Base class for all simplelocalize-download errors."""

class AuthError(SimpleLocalizeError):
    """Invalid API credentials (HTTP 401/403). Has a ``status`` attribute."""

    status: int

class ApiError(SimpleLocalizeError):
    """The SimpleLocalize API returned an error.

    ``status`` is the HTTP status code, or ``None`` when the error was not tied
    to a specific response (e.g. a malformed response body).
    """

    status: int | None

class NetworkError(SimpleLocalizeError):
    """A network/transport failure."""

class DownloadFailedError(SimpleLocalizeError):
    """One or more files failed to download; inspect ``report``."""

    report: DownloadReport

def download(
    api_key: str,
    download_path: str,
    *,
    download_format: str | None = ...,
    language_keys: list[str] | None = ...,
    download_options: list[str] | None = ...,
    download_sort: str | None = ...,
    namespace: str | None = ...,
    tags: list[str] | None = ...,
    customer_id: str | None = ...,
    base_url: str | None = ...,
    concurrency: int = ...,
    request_timeout_secs: float | None = ...,
    on_event: Callable[[dict[str, Any]], None] | None = ...,
) -> DownloadReport:
    """Download SimpleLocalize translations (JSON only).

    Raises ``AuthError``, ``ApiError`` or ``NetworkError`` on a pre-flight
    failure, and ``DownloadFailedError`` (carrying a ``report``) when the batch
    finishes with per-file failures.
    """
