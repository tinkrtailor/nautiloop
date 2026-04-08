#!/usr/bin/env python3
"""Mock example HTTP server for the Nautiloop parity harness (FR-11).

Binds port 80, 8080, and 9999. Port 80 and 8080 serve identical
handlers (the `with_port` egress case reaches :8080 through the
sidecar). Port 9999 is the introspection API per FR-13.

Health probes on /_healthz are NEVER logged.
"""
import asyncio
import base64
import sys
import threading
from dataclasses import dataclass
from typing import List

from hypercorn.asyncio import serve
from hypercorn.config import Config
from starlette.applications import Starlette
from starlette.requests import Request
from starlette.responses import JSONResponse, PlainTextResponse, Response
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

    def record(self, method, path, headers, body, client_host):
        with self._lock:
            self._next_id += 1
            self._entries.append(
                ObservedRequest(
                    method=method,
                    path=path,
                    host_header=headers.get("host", ""),
                    headers={k.lower(): v for k, v in headers.items()},
                    body_b64=base64.b64encode(body).decode(),
                    source_ip=client_host,
                    timestamp="",
                    id=self._next_id,
                )
            )

    def snapshot(self):
        with self._lock:
            return [e.__dict__ for e in self._entries]

    def reset(self):
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


async def foo(request: Request) -> Response:
    await _record(request)
    return PlainTextResponse("mock-example-foo\n")


async def redirect(request: Request) -> Response:
    await _record(request)
    return Response(
        status_code=302,
        headers={"location": "/foo"},
    )


async def not_found(request: Request) -> Response:
    await _record(request)
    return JSONResponse({"error": "not found"}, status_code=404)


async def introspect_logs(request: Request) -> Response:
    return JSONResponse(STORE.snapshot())


async def introspect_reset(request: Request) -> Response:
    STORE.reset()
    return JSONResponse({"ok": True})


def build_http_app() -> Starlette:
    return Starlette(
        routes=[
            Route("/_healthz", health, methods=["GET"]),
            Route("/foo", foo, methods=["GET"]),
            Route("/redirect", redirect, methods=["GET"]),
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
    http_config_80 = Config()
    http_config_80.bind = ["0.0.0.0:80"]
    http_config_80.accesslog = "-"

    http_config_8080 = Config()
    http_config_8080.bind = ["0.0.0.0:8080"]
    http_config_8080.accesslog = "-"

    intro_config = Config()
    intro_config.bind = ["0.0.0.0:9999"]
    intro_config.accesslog = "-"

    app = build_http_app()
    intro_app = build_introspection_app()
    await asyncio.gather(
        serve(app, http_config_80),
        serve(app, http_config_8080),
        serve(intro_app, intro_config),
    )


if __name__ == "__main__":
    try:
        asyncio.run(run())
    except KeyboardInterrupt:
        sys.exit(0)
