# Raw TCP echo server (FR-12). asyncio only — no external pips.

FROM python:3.12-slim
WORKDIR /app
COPY sidecar/tests/parity/fixtures/mock-tcp-echo/server.py ./server.py
CMD ["python3", "server.py"]
