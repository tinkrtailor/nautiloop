#!/usr/bin/env python3
"""Raw TCP echo server (FR-12).

Binds 0.0.0.0:443. For every accepted connection, reads until the
client closes, echoing every chunk back. No higher-level protocol,
no introspection — byte counts are captured by the harness driver
through its own client side.
"""
import asyncio
import sys


async def handle(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
    peer = writer.get_extra_info("peername")
    try:
        while True:
            chunk = await reader.read(4096)
            if not chunk:
                break
            writer.write(chunk)
            await writer.drain()
    except Exception as e:
        print(f"echo error from {peer}: {e}", file=sys.stderr)
    finally:
        try:
            writer.close()
            await writer.wait_closed()
        except Exception:
            pass


async def run() -> None:
    server = await asyncio.start_server(handle, "0.0.0.0", 443)
    print("mock-tcp-echo: listening on :443", file=sys.stderr)
    async with server:
        await server.serve_forever()


if __name__ == "__main__":
    try:
        asyncio.run(run())
    except KeyboardInterrupt:
        sys.exit(0)
