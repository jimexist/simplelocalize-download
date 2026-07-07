"""Command-line interface for ``simplelocalize-download``.

A near drop-in replacement for the SimpleLocalize Java CLI's ``download`` command
(JSON only). Existing invocations port by swapping the binary name: the Java
camelCase flags (``--apiKey``, ``--downloadPath``, …) are accepted as aliases
alongside the kebab-case names.
"""

from __future__ import annotations

import sys
import time
from typing import Annotated, Any

import typer
from rich.console import Console
from rich.progress import (
    BarColumn,
    MofNCompleteColumn,
    Progress,
    SpinnerColumn,
    TaskID,
    TextColumn,
    TimeElapsedColumn,
)

from . import __version__
from ._core import (
    ApiError,
    AuthError,
    DownloadFailedError,
    DownloadReport,
    NetworkError,
)
from ._core import download as _core_download
from .logging_setup import configure_logging

SUPPORTED_FORMAT = "single-language-json"

# Exit codes.
EXIT_USAGE = 2
EXIT_AUTH = 3
EXIT_API = 4
EXIT_PARTIAL = 5

console = Console(stderr=True)

app = typer.Typer(
    name="simplelocalize-download",
    help="Resilient downloader for SimpleLocalize translations (JSON only).",
    add_completion=False,
    no_args_is_help=True,
)


def _version_callback(value: bool) -> None:
    if value:
        typer.echo(__version__)
        raise typer.Exit()


@app.callback()
def main(
    version: Annotated[
        bool,
        typer.Option("--version", callback=_version_callback, is_eager=True, help="Show version."),
    ] = False,
) -> None:
    """SimpleLocalize download CLI."""


def _split(value: str | None) -> list[str] | None:
    """Parse a comma-separated option value into a list (or None if empty)."""
    if not value:
        return None
    items = [part.strip() for part in value.split(",") if part.strip()]
    return items or None


class _ProgressReporter:
    """Drives a rich progress bar from download events (when on a TTY)."""

    def __init__(self, enabled: bool) -> None:
        self.enabled = enabled
        self.progress: Progress | None = None
        self.task: TaskID | None = None

    def __enter__(self) -> _ProgressReporter:
        if self.enabled:
            self.progress = Progress(
                SpinnerColumn(),
                TextColumn("[progress.description]{task.description}"),
                BarColumn(),
                MofNCompleteColumn(),
                TimeElapsedColumn(),
                console=console,
                transient=True,
            )
            self.progress.start()
        return self

    def __exit__(self, *exc: object) -> None:
        if self.progress is not None:
            self.progress.stop()

    def on_event(self, event: dict[str, Any]) -> None:
        if self.progress is None:
            return
        kind = event.get("type")
        if kind == "started":
            self.task = self.progress.add_task("Downloading", total=int(event["total"]))
        elif kind in ("file_done", "file_failed") and self.task is not None:
            self.progress.advance(self.task)


def _print_summary(report: DownloadReport, started_at: float) -> None:
    elapsed = time.monotonic() - started_at
    total_bytes = sum(f.bytes for f in report.downloaded)
    console.print(
        f"Downloaded [bold]{len(report.downloaded)}[/] file(s), "
        f"{total_bytes} bytes in {elapsed:.1f}s"
    )
    if report.failed:
        console.print(f"[red]{len(report.failed)} file(s) failed:[/]")
        for failed in report.failed:
            location = f"{failed.language or '?'}/{failed.namespace or '?'}"
            console.print(f"  [red]{location}[/]: {failed.error}")


@app.command()
def download(
    api_key: Annotated[
        str,
        typer.Option(
            "--api-key",
            "--apiKey",
            envvar="SIMPLELOCALIZE_API_KEY",
            show_envvar=True,
            help="SimpleLocalize project API key.",
        ),
    ],
    download_path: Annotated[
        str,
        typer.Option(
            "--download-path",
            "--downloadPath",
            help="Output path template, e.g. ./json/{lang}/{ns}.json",
        ),
    ],
    download_format: Annotated[
        str,
        typer.Option("--download-format", "--downloadFormat", help="Only single-language-json."),
    ] = SUPPORTED_FORMAT,
    download_options: Annotated[
        str | None,
        typer.Option(
            "--download-options", "--downloadOptions", help="Comma-separated, e.g. WRITE_NESTED."
        ),
    ] = None,
    download_sort: Annotated[
        str | None,
        typer.Option("--download-sort", "--downloadSort", help="e.g. LEXICOGRAPHICAL."),
    ] = None,
    language_key: Annotated[
        str | None,
        typer.Option(
            "--language-key", "--downloadLanguageKey", help="Comma-separated language keys."
        ),
    ] = None,
    namespace: Annotated[
        str | None,
        typer.Option("--namespace", "--downloadNamespace", help="Namespace filter."),
    ] = None,
    tags: Annotated[
        str | None,
        typer.Option("--tags", "--downloadTags", help="Comma-separated tag filter."),
    ] = None,
    customer_id: Annotated[
        str | None,
        typer.Option("--customer-id", "--downloadCustomerId", help="Customer ID filter."),
    ] = None,
    base_url: Annotated[
        str,
        typer.Option("--base-url", "--baseUrl", help="API base URL."),
    ] = "https://api.simplelocalize.io",
    concurrency: Annotated[
        int,
        typer.Option("--concurrency", min=1, help="Max concurrent file downloads."),
    ] = 8,
    timeout: Annotated[
        float | None,
        typer.Option("--timeout", help="Per-request timeout in seconds."),
    ] = None,
    verbose: Annotated[bool, typer.Option("--verbose", "-v", help="Debug logging.")] = False,
    quiet: Annotated[bool, typer.Option("--quiet", "-q", help="Warnings and errors only.")] = False,
    no_progress: Annotated[
        bool, typer.Option("--no-progress", help="Disable the progress bar.")
    ] = False,
) -> None:
    """Download translations from SimpleLocalize."""
    level = "DEBUG" if verbose else "WARNING" if quiet else "INFO"
    configure_logging(level)

    if download_format != SUPPORTED_FORMAT:
        console.print(
            f"[red]error:[/] unsupported --download-format {download_format!r}; "
            f"this tool only supports {SUPPORTED_FORMAT!r}."
        )
        raise typer.Exit(EXIT_USAGE)

    show_progress = not no_progress and not quiet and sys.stderr.isatty()
    started_at = time.monotonic()

    try:
        with _ProgressReporter(show_progress) as reporter:
            report = _core_download(
                api_key=api_key,
                download_path=download_path,
                download_format=download_format,
                language_keys=_split(language_key),
                download_options=_split(download_options),
                download_sort=download_sort,
                namespace=namespace,
                tags=_split(tags),
                customer_id=customer_id,
                base_url=base_url,
                concurrency=concurrency,
                request_timeout_secs=timeout,
                on_event=reporter.on_event if show_progress else None,
            )
    except AuthError as err:
        console.print(f"[red]authentication failed:[/] {err}")
        raise typer.Exit(EXIT_AUTH) from err
    except (ApiError, NetworkError) as err:
        console.print(f"[red]download failed:[/] {err}")
        raise typer.Exit(EXIT_API) from err
    except DownloadFailedError as err:
        _print_summary(err.report, started_at)
        raise typer.Exit(EXIT_PARTIAL) from err

    _print_summary(report, started_at)


if __name__ == "__main__":
    app()
