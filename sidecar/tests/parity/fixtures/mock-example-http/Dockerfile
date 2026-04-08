# Mock example HTTP server (FR-11). Dual-port (80 + 8080).

FROM python:3.12-slim
WORKDIR /app
RUN pip install --no-cache-dir \
    hypercorn==0.17.3 \
    starlette==0.38.6

COPY sidecar/tests/parity/fixtures/mock-example-http/server.py ./server.py

CMD ["python3", "server.py"]
