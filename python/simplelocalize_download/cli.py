"""Command-line interface for ``simplelocalize-download``.

This is a placeholder app that will grow the real ``download`` command in a
later change. It exists now so the ``simplelocalize-download`` console script
entry point resolves and ``--version`` works end to end.
"""

from __future__ import annotations

import typer

from . import __version__

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
    version: bool = typer.Option(
        False,
        "--version",
        help="Show the version and exit.",
        callback=_version_callback,
        is_eager=True,
    ),
) -> None:
    """SimpleLocalize download CLI."""


if __name__ == "__main__":
    app()
