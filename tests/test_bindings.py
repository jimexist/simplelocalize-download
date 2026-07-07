"""Tests for the PyO3 binding surface (``simplelocalize_download.download``)."""

from __future__ import annotations

import threading
import time
from pathlib import Path

import pytest

import simplelocalize_download as sld
from tests.conftest import MockState


def _template(tmp_path: Path) -> str:
    return str(tmp_path / "{lang}" / "{ns}.json")


def test_downloads_files(mock_server: tuple[str, MockState], tmp_path: Path) -> None:
    base, state = mock_server
    state.add("en", "common", b'{"a":1}')
    state.add("ja", "common", b'{"b":2}')

    report = sld.download(api_key="k", download_path=_template(tmp_path), base_url=base)

    assert report.is_success()
    assert len(report.downloaded) == 2
    assert report.failed == []
    assert (tmp_path / "en" / "common.json").read_bytes() == b'{"a":1}'
    assert (tmp_path / "ja" / "common.json").read_bytes() == b'{"b":2}'


def test_unsupported_format_raises_value_error(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="single-language-json"):
        sld.download(
            api_key="k",
            download_path=_template(tmp_path),
            download_format="yaml",
        )


def test_auth_error(mock_server: tuple[str, MockState], tmp_path: Path) -> None:
    base, state = mock_server
    state.add("en", "common", b"{}")
    state.list_status = 401

    with pytest.raises(sld.AuthError) as excinfo:
        sld.download(api_key="bad", download_path=_template(tmp_path), base_url=base)
    assert excinfo.value.status == 401


def test_malformed_response_is_api_error_without_status(
    mock_server: tuple[str, MockState], tmp_path: Path
) -> None:
    base, state = mock_server
    state.list_body = b"this is not json"

    with pytest.raises(sld.ApiError) as excinfo:
        sld.download(api_key="k", download_path=_template(tmp_path), base_url=base)
    # `.status` is always present on ApiError, but None when not tied to a code.
    assert excinfo.value.status is None


@pytest.mark.parametrize("bad", [-1.0, float("nan"), float("inf")])
def test_invalid_timeout_raises_value_error(
    mock_server: tuple[str, MockState], tmp_path: Path, bad: float
) -> None:
    base, state = mock_server
    state.add("en", "common", b"{}")
    with pytest.raises(ValueError):
        sld.download(
            api_key="k",
            download_path=_template(tmp_path),
            base_url=base,
            request_timeout_secs=bad,
        )


def test_partial_failure_raises_with_report(
    mock_server: tuple[str, MockState], tmp_path: Path
) -> None:
    base, state = mock_server
    state.add("en", "ok", b"{}")
    state.add("ja", "bad", b"{}")
    state.file_status[("ja", "bad")] = 404

    with pytest.raises(sld.DownloadFailedError) as excinfo:
        sld.download(api_key="k", download_path=_template(tmp_path), base_url=base)

    report = excinfo.value.report
    assert len(report.downloaded) == 1
    assert len(report.failed) == 1
    assert report.failed[0].namespace == "bad"
    assert (tmp_path / "en" / "ok.json").exists()
    assert not (tmp_path / "ja" / "bad.json").exists()


def test_progress_callback_fires_per_file(
    mock_server: tuple[str, MockState], tmp_path: Path
) -> None:
    base, state = mock_server
    state.add("en", "a", b"{}")
    state.add("en", "b", b"{}")

    events: list[dict[str, object]] = []
    sld.download(
        api_key="k",
        download_path=_template(tmp_path),
        base_url=base,
        on_event=events.append,
    )

    types = [e["type"] for e in events]
    assert types.count("file_done") == 2
    assert "started" in types
    assert "finished" in types


def test_raising_callback_does_not_abort(
    mock_server: tuple[str, MockState], tmp_path: Path
) -> None:
    base, state = mock_server
    state.add("en", "a", b"{}")
    state.add("en", "b", b"{}")

    def boom(_event: dict[str, object]) -> None:
        raise RuntimeError("callback exploded")

    report = sld.download(
        api_key="k",
        download_path=_template(tmp_path),
        base_url=base,
        on_event=boom,
    )
    assert len(report.downloaded) == 2


def test_gil_released_during_download(mock_server: tuple[str, MockState], tmp_path: Path) -> None:
    base, state = mock_server
    state.add("en", "slow", b"{}")
    state.file_delay = 0.3  # keep the Rust side busy for ~300ms

    counter = 0
    stop = threading.Event()

    def spin() -> None:
        nonlocal counter
        while not stop.is_set():
            counter += 1  # CPU-bound Python work; only runs if it holds the GIL

    worker = threading.Thread(target=spin)
    worker.start()
    try:
        sld.download(api_key="k", download_path=_template(tmp_path), base_url=base)
    finally:
        stop.set()
        worker.join()
        time.sleep(0)

    # If the GIL were held for the whole download, the spin loop would be starved.
    assert counter > 1000, f"spin thread only advanced {counter} times"
