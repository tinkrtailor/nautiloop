#!/usr/bin/env python3
"""Mock Anthropic HTTPS server for the Nautiloop parity harness (FR-8).

Same 3-port topology as mock-openai (see that server.py for the
introspection contract). Responds only to `POST /v1/messages` and
`GET /_healthz`; everything else returns 404.
"""
import asyncio
import base64
import json
import os
import sys
import threading
from dataclasses import dataclass
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
                timestamp="",
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
    return JSONResponse({"ok": True})


async def v1_messages(request: Request) -> Response:
    body = await _record(request)
    try:
        payload = json.loads(body.decode() or "{}")
    except json.JSONDecodeError:
        payload = {}

    if payload.get("stream") is True:
        return StreamingResponse(_anthropic_sse_stream(), media_type="text/event-stream")

    return JSONResponse(
        {
            "id": "msg_parity",
            "type": "message",
            "role": "assistant",
            "model": payload.get("model", "claude-3-5-sonnet"),
            "content": [{"type": "text", "text": "pong"}],
            "stop_reason": "end_turn",
        }
    )


async def _anthropic_sse_stream():
    """Yield 3 SSE events spaced 100ms apart — same timing contract
    as the openai mock (FR-8)."""
    for i in range(3):
        chunk = {
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": f"tok{i}"},
        }
        yield f"data: {json.dumps(chunk)}\n\n".encode()
        await asyncio.sleep(0.1)
    yield b"data: [DONE]\n\n"


# ---------- introspection ----------


async def introspect_logs(request: Request) -> Response:
    return JSONResponse(STORE.snapshot())


async def introspect_reset(request: Request) -> Response:
    STORE.reset()
    return JSONResponse({"ok": True})


def build_https_app() -> Starlette:
    return Starlette(
        routes=[
            Route("/v1/messages", v1_messages, methods=["POST"]),
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
