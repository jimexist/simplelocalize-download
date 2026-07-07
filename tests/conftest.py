"""A configurable in-process mock of the SimpleLocalize download API.

Serves ``GET /cli/v2/download`` (the file list) and per-file payload endpoints
that stand in for presigned URLs, with hooks for injecting failures, delays and
transient errors. Shared by the binding, CLI, and end-to-end test suites.
"""

from __future__ import annotations

import json
import threading
import time
from collections.abc import Iterator
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest


@dataclass
class MockState:
    """Mutable configuration for the mock server, tweaked per test."""

    files: dict[tuple[str, str], bytes] = field(default_factory=dict)
    """(language, namespace) -> file content."""
    list_status: int = 200
    """Status returned by the list endpoint (non-200 => error)."""
    list_body: bytes | None = None
    """Raw 200 body for the list endpoint (e.g. malformed JSON); None => generate."""
    file_status: dict[tuple[str, str], int] = field(default_factory=dict)
    """Per-file status override."""
    transient_failures: dict[tuple[str, str], int] = field(default_factory=dict)
    """Per-file count of 500s to emit before succeeding."""
    file_delay: float = 0.0
    """Seconds to sleep before serving each file (simulate slow network)."""
    base_url: str = ""

    def add(self, language: str, namespace: str, content: bytes) -> None:
        self.files[(language, namespace)] = content


def _make_handler(state: MockState) -> type[BaseHTTPRequestHandler]:
    lock = threading.Lock()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args: object) -> None:  # silence stderr noise
            pass

        def _send(
            self, status: int, body: bytes = b"", content_type: str = "application/json"
        ) -> None:
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if body:
                self.wfile.write(body)

        def do_GET(self) -> None:  # noqa: N802 (http.server API)
            if self.path.startswith("/cli/v2/download"):
                self._serve_list()
            elif self.path.startswith("/files/"):
                self._serve_file()
            else:
                self._send(404)

        def _serve_list(self) -> None:
            if state.list_status != 200:
                self._send(state.list_status, json.dumps({"msg": "list failed"}).encode())
                return
            if state.list_body is not None:
                self._send(200, state.list_body)
                return
            entries = [
                {
                    "url": f"{state.base_url}/files/{lang}/{ns}.json",
                    "language": lang,
                    "namespace": ns,
                }
                for (lang, ns) in state.files
            ]
            self._send(200, json.dumps({"files": entries}).encode())

        def _serve_file(self) -> None:
            rest = self.path[len("/files/") :]
            lang, filename = rest.split("/", 1)
            namespace = filename.removesuffix(".json")
            key = (lang, namespace)

            if state.file_delay:
                time.sleep(state.file_delay)

            with lock:
                remaining = state.transient_failures.get(key, 0)
                if remaining > 0:
                    state.transient_failures[key] = remaining - 1
                    self._send(500)
                    return

            status = state.file_status.get(key, 200)
            if status != 200:
                self._send(status)
                return
            self._send(200, state.files.get(key, b""))

    return Handler


@pytest.fixture
def mock_server() -> Iterator[tuple[str, MockState]]:
    """Yield ``(base_url, state)``; mutate ``state`` to shape responses."""
    state = MockState()
    server = ThreadingHTTPServer(("127.0.0.1", 0), _make_handler(state))
    state.base_url = f"http://127.0.0.1:{server.server_address[1]}"
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield state.base_url, state
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
