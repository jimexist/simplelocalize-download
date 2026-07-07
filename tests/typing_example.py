"""A strictly-typed usage snippet, type-checked by mypy to exercise the stubs."""

from __future__ import annotations

from typing import Any

import simplelocalize_download as sld


def run() -> int:
    def on_event(event: dict[str, Any]) -> None:
        print(event["type"])

    try:
        report: sld.DownloadReport = sld.download(
            api_key="key",
            download_path="./json/{lang}/{ns}.json",
            download_options=["WRITE_NESTED"],
            download_sort="LEXICOGRAPHICAL",
            language_keys=["en", "ja"],
            concurrency=4,
            on_event=on_event,
        )
    except sld.AuthError as err:
        print(err.status)
        return 3
    except sld.DownloadFailedError as err:
        for failed in err.report.failed:
            print(failed.url, failed.error)
        return 5

    total: int = sum(f.bytes for f in report.downloaded)
    return 0 if total >= 0 else 1
