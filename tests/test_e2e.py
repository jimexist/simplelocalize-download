"""End-to-end tests: CLI -> PyO3 bindings -> Rust core against the mock server.

Uses a scaled-down version of the real consumer's shape (pixai: 14 languages x
33 namespaces of nested JSON).
"""

from __future__ import annotations

import json
from pathlib import Path

from typer.testing import CliRunner

from simplelocalize_download.cli import app
from tests.conftest import MockState

runner = CliRunner()

LANGS = ["en", "de", "es", "fr", "ja"]
NAMESPACES = [
    "common",
    "auth",
    "onboarding",
    "errors",
    "billing",
    "generation",
    "model",
    "artwork",
    "settings",
    "landing",
]


def _seed(state: MockState) -> None:
    for lang in LANGS:
        for ns in NAMESPACES:
            content = json.dumps({"lang": lang, "ns": ns, "key": f"{lang}.{ns}"}).encode()
            state.add(lang, ns, content)


def _template(tmp_path: Path) -> str:
    return str(tmp_path / "{lang}" / "{ns}.json")


def _invoke(tmp_path: Path, base: str, *extra: str) -> object:
    return runner.invoke(
        app,
        [
            "download",
            "--api-key",
            "k",
            "--download-path",
            _template(tmp_path),
            "--base-url",
            base,
            "--no-progress",
            *extra,
        ],
    )


def _tmp_litter(root: Path) -> list[Path]:
    return [p for p in root.rglob("*") if p.is_file() and ".tmp-" in p.name]


def _assert_tree(tmp_path: Path) -> None:
    for lang in LANGS:
        for ns in NAMESPACES:
            path = tmp_path / lang / f"{ns}.json"
            assert path.exists(), f"missing {path}"
            data = json.loads(path.read_text())
            assert data == {"lang": lang, "ns": ns, "key": f"{lang}.{ns}"}


def test_green_path_full_tree(mock_server: tuple[str, MockState], tmp_path: Path) -> None:
    base, state = mock_server
    _seed(state)
    result = _invoke(tmp_path, base)
    assert result.exit_code == 0, result.output
    _assert_tree(tmp_path)
    assert not _tmp_litter(tmp_path)


def test_flaky_server_still_succeeds(mock_server: tuple[str, MockState], tmp_path: Path) -> None:
    base, state = mock_server
    _seed(state)
    # ~20% of files fail once with 500 before succeeding — retries must recover.
    all_keys = [(lang, ns) for lang in LANGS for ns in NAMESPACES]
    for key in all_keys[::5]:
        state.transient_failures[key] = 1

    result = _invoke(tmp_path, base)
    assert result.exit_code == 0, result.output
    _assert_tree(tmp_path)
    assert not _tmp_litter(tmp_path)


def test_hard_failure_exits_5_others_present(
    mock_server: tuple[str, MockState], tmp_path: Path
) -> None:
    base, state = mock_server
    _seed(state)
    state.file_status[("fr", "billing")] = 404

    result = _invoke(tmp_path, base)
    assert result.exit_code == 5, result.output
    assert not (tmp_path / "fr" / "billing.json").exists()
    # Everything else still landed.
    assert (tmp_path / "en" / "common.json").exists()
    assert (tmp_path / "ja" / "landing.json").exists()
    assert not _tmp_litter(tmp_path)


def test_auth_failure_exits_3_no_files(mock_server: tuple[str, MockState], tmp_path: Path) -> None:
    base, state = mock_server
    _seed(state)
    state.list_status = 401

    result = _invoke(tmp_path, base)
    assert result.exit_code == 3, result.output
    assert not list(tmp_path.rglob("*.json"))


def test_rerun_overwrites_cleanly(mock_server: tuple[str, MockState], tmp_path: Path) -> None:
    base, state = mock_server
    _seed(state)
    assert _invoke(tmp_path, base).exit_code == 0
    # Second run over the populated tree.
    result = _invoke(tmp_path, base)
    assert result.exit_code == 0, result.output
    _assert_tree(tmp_path)
    assert not _tmp_litter(tmp_path)


def test_large_batch_concurrency(mock_server: tuple[str, MockState], tmp_path: Path) -> None:
    base, state = mock_server
    # 200 files: sanity that concurrency 8 completes without fd exhaustion.
    for i in range(200):
        state.add("en", f"ns{i:03d}", json.dumps({"i": i}).encode())

    result = _invoke(tmp_path, base, "--concurrency", "8")
    assert result.exit_code == 0, result.output
    files = list((tmp_path / "en").glob("*.json"))
    assert len(files) == 200
    assert not _tmp_litter(tmp_path)
