"""Smoke tests: the package imports and its version matches the crate."""

from __future__ import annotations

from pathlib import Path

import tomllib

import simplelocalize_download


def test_import() -> None:
    assert simplelocalize_download.__version__


def test_version_matches_cargo() -> None:
    cargo = Path(__file__).resolve().parent.parent / "Cargo.toml"
    data = tomllib.loads(cargo.read_text())
    assert simplelocalize_download.__version__ == data["package"]["version"]
