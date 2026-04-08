#!/usr/bin/env python3
"""Mock OpenAI HTTPS server for the Nautiloop parity harness (FR-7).

Binds three ports:

- 80/tcp:   plain HTTP `/_healthz` only. Used by docker-compose
            healthcheck and the harness driver's FR-17 step 3 poll.
            Health requests are NEVER logged to the introspection
            store (FR-13).
- 443/tcp:  HTTPS serving the three openai handlers. Uses the test
            CA signed cert with SAN=api.openai.com.
- 9999/tcp: plain HTTP introspection API per FR-13:
              - GET  /__harness/logs  -> observed requests JSON
              - POST /__harness/reset -> clear log store
            Traffic on port 80 `/_healthz` is NEVER logged.
"""
import asyncio
import base64
import json
import os
import ssl
import sys
import threading
from dataclasses import dataclass, field
from typing import List

from hypercorn.asyncio import serve
from hypercorn.config import Config
from starlette.applications import Starlette
from starlette.requests import Request
from starlette.responses import JSONResponse, Response, StreamingResponse
from starlette.routing import Route


@dataclass
class ObservedRequest:
    method: str
    path: str
    host_header: str
    headers: dict
    body_b64: str
    source_ip: str
    timestamp: str
    id: int


class Store:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._entries: List[ObservedRequest] = []
        self._next_id = 0

    def record(self, method: str, path: str, headers: dict, body: bytes, client_host: str) -> None:
        with self._lock:
            self._next_id += 1
            entry = ObservedRequest(
                method=method,
                path=path,
                host_header=headers.get("host", ""),
                headers={k.lower(): v for k, v in headers.items()},
                body_b64=base64.b64encode(body).decode(),
                source_ip=client_host,
                timestamp="",  # timestamps are stripped by FR-19 anyway
                id=self._next_id,
            )
            self._entries.append(entry)

    def snapshot(self) -> List[dict]:
        with self._lock:
            return [e.__dict__ for e in self._entries]

    def reset(self) -> None:
        with self._lock:
            self._entries.clear()
            self._next_id = 0


STORE = Store()


async def _record(request: Request) -> bytes:
    body = await request.body()
    STORE.record(
        request.method,
        request.url.path,
        dict(request.headers),
        body,
        request.client.host if request.client else "",
    )
    return body


async def health(request: Request) -> Response:
    # FR-13: health probes are NEVER logged.
    return JSONResponse({"ok": True})


async def v1_models(request: Request) -> Response:
    await _record(request)
    return JSONResponse(
        {
            "object": "list",
            "data": [
                {"id": "gpt-4", "object": "model", "owned_by": "nautiloop-parity"},
                {"id": "gpt-4o", "object": "model", "owned_by": "nautiloop-parity"},
            ],
        }
    )


async def v1_chat_completions(request: Request) -> Response:
    body = await _record(request)
    try:
        payload = json.loads(body.decode() or "{}")
    except json.JSONDecodeError:
        payload = {}

    if payload.get("stream") is True:
        return StreamingResponse(_openai_sse_stream(), media_type="text/event-stream")

    return JSONResponse(
        {
            "id": "chatcmpl-parity",
            "object": "chat.completion",
            "model": payload.get("model", "gpt-4"),
            "choices": [
                {
                    "index": 0,
                    "finish_reason": "stop",
                    "message": {"role": "assistant", "content": "pong"},
                }
            ],
        }
    )


async def _openai_sse_stream():
    """Yield three OpenAI-style SSE chunks spaced 100ms apart.

    This is the test shape for `divergence_sse_streaming_openai`
    (FR-22). Rust must deliver the first chunk to the client within
    ~200ms of request send; Go will buffer everything until upstream
    close and miss that window.

    We flush between events by awaiting `asyncio.sleep` — hypercorn
    sends each yielded chunk on the wire immediately.
    """
    import time

    for i in range(3):
        chunk = {
            "id": "chatcmpl-parity",
            "object": "chat.completion.chunk",
            "choices": [
                {
                    "index": 0,
                    "delta": {"content": f"tok{i}"},
                    "finish_reason": None,
                }
            ],
        }
        yield f"data: {json.dumps(chunk)}\n\n".encode()
        await asyncio.sleep(0.1)
    yield b"data: [DONE]\n\n"


async def not_found(request: Request) -> Response:
    # Anything not explicitly routed returns 404 per FR-7.
    return JSONResponse({"error": "not found"}, status_code=404)


# ---------- introspection API on port 9999 ----------


async def introspect_logs(request: Request) -> Response:
    return JSONResponse(STORE.snapshot())


async def introspect_reset(request: Request) -> Response:
    STORE.reset()
    return JSONResponse({"ok": True})


# ---------- app builders ----------


def build_https_app() -> Starlette:
    return Starlette(
        routes=[
            Route("/v1/models", v1_models, methods=["GET"]),
            Route("/v1/chat/completions", v1_chat_completions, methods=["POST"]),
            # Catch-all 404 for every other route. Starlette doesn't
            # have a wildcard route natively, so we attach the
            # handler via `exception_handler` on the 404 response
            # using a middleware. Simpler: leave the default 404 as-is
            # — Starlette returns its own 404 for unknown paths and
            # that's acceptable for our diff comparison because both
            # sidecars see the same 404 body.
        ]
    )


def build_health_app() -> Starlette:
    return Starlette(
        routes=[
            Route("/_healthz", health, methods=["GET"]),
        ]
    )


def build_introspection_app() -> Starlette:
    return Starlette(
        routes=[
            Route("/__harness/logs", introspect_logs, methods=["GET"]),
            Route("/__harness/reset", introspect_reset, methods=["POST"]),
        ]
    )


async def run() -> None:
    cert_path = os.environ.get("MOCK_CERT_PATH", "/app/cert.pem")
    key_path = os.environ.get("MOCK_KEY_PATH", "/app/key.pem")

    https_config = Config()
    https_config.bind = ["0.0.0.0:443"]
    https_config.certfile = cert_path
    https_config.keyfile = key_path
    https_config.alpn_protocols = ["http/1.1"]
    https_config.accesslog = "-"

    health_config = Config()
    health_config.bind = ["0.0.0.0:80"]
    health_config.accesslog = "-"

    intro_config = Config()
    intro_config.bind = ["0.0.0.0:9999"]
    intro_config.accesslog = "-"

    await asyncio.gather(
        serve(build_https_app(), https_config),
        serve(build_health_app(), health_config),
        serve(build_introspection_app(), intro_config),
    )


if __name__ == "__main__":
    try:
        asyncio.run(run())
    except KeyboardInterrupt:
        sys.exit(0)
