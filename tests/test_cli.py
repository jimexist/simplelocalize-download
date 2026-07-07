"""Tests for the Typer CLI."""

from __future__ import annotations

from pathlib import Path

import pytest
from typer.testing import CliRunner

from simplelocalize_download.cli import app
from tests.conftest import MockState

runner = CliRunner()


def _path_template(tmp_path: Path) -> str:
    return str(tmp_path / "{lang}" / "{ns}.json")


def test_download_happy_path(mock_server: tuple[str, MockState], tmp_path: Path) -> None:
    base, state = mock_server
    state.add("en", "common", b'{"a":1}')
    state.add("ja", "common", b'{"b":2}')

    result = runner.invoke(
        app,
        [
            "download",
            "--api-key",
            "k",
            "--download-path",
            _path_template(tmp_path),
            "--base-url",
            base,
        ],
    )
    assert result.exit_code == 0, result.output
    assert (tmp_path / "en" / "common.json").read_bytes() == b'{"a":1}'
    assert (tmp_path / "ja" / "common.json").read_bytes() == b'{"b":2}'


def test_camelcase_aliases_match_java_cli(
    mock_server: tuple[str, MockState], tmp_path: Path
) -> None:
    """The pixai invocation with only the binary name swapped."""
    base, state = mock_server
    state.add("en", "common", b"{}")

    result = runner.invoke(
        app,
        [
            "download",
            "--apiKey",
            "k",
            "--downloadFormat",
            "single-language-json",
            "--downloadPath",
            _path_template(tmp_path),
            "--downloadOptions",
            "WRITE_NESTED",
            "--downloadSort",
            "LEXICOGRAPHICAL",
            "--baseUrl",
            base,
        ],
    )
    assert result.exit_code == 0, result.output
    assert (tmp_path / "en" / "common.json").exists()


def test_api_key_from_env(
    mock_server: tuple[str, MockState], tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    base, state = mock_server
    state.add("en", "common", b"{}")
    monkeypatch.setenv("SIMPLELOCALIZE_API_KEY", "from-env")

    result = runner.invoke(
        app,
        ["download", "--download-path", _path_template(tmp_path), "--base-url", base],
    )
    assert result.exit_code == 0, result.output
    assert (tmp_path / "en" / "common.json").exists()


def test_missing_api_key_is_usage_error(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("SIMPLELOCALIZE_API_KEY", raising=False)
    result = runner.invoke(app, ["download", "--download-path", _path_template(tmp_path)])
    assert result.exit_code == 2


def test_non_json_format_rejected(tmp_path: Path) -> None:
    result = runner.invoke(
        app,
        [
            "download",
            "--api-key",
            "k",
            "--download-path",
            _path_template(tmp_path),
            "--download-format",
            "yaml",
        ],
    )
    assert result.exit_code == 2
    assert "single-language-json" in result.output


def test_partial_failure_exits_5(mock_server: tuple[str, MockState], tmp_path: Path) -> None:
    base, state = mock_server
    state.add("en", "ok", b"{}")
    state.add("ja", "bad", b"{}")
    state.file_status[("ja", "bad")] = 404

    result = runner.invoke(
        app,
        [
            "download",
            "--api-key",
            "k",
            "--download-path",
            _path_template(tmp_path),
            "--base-url",
            base,
        ],
    )
    assert result.exit_code == 5, result.output
    assert (tmp_path / "en" / "ok.json").exists()


def test_auth_error_exits_3(mock_server: tuple[str, MockState], tmp_path: Path) -> None:
    base, state = mock_server
    state.add("en", "common", b"{}")
    state.list_status = 401

    result = runner.invoke(
        app,
        [
            "download",
            "--api-key",
            "bad",
            "--download-path",
            _path_template(tmp_path),
            "--base-url",
            base,
        ],
    )
    assert result.exit_code == 3, result.output
