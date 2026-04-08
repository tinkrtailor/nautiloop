# Raw TCP echo server (FR-12). asyncio only — no external pips needed
# by the server, but FR-10 says every mock image installs the full
# reproducibility set so any mock can take on any role without a pip
# diff.

FROM python:3.12-slim
WORKDIR /app
RUN pip install --no-cache-dir \
    hypercorn==0.17.3 \
    uvicorn==0.30.6 \
    starlette==0.38.6 \
    paramiko==3.4.0
COPY sidecar/tests/parity/fixtures/mock-tcp-echo/server.py ./server.py
CMD ["python3", "server.py"]
