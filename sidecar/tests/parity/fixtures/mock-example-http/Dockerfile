# Mock example HTTP server (FR-11). Dual-port (80 + 8080).

FROM python:3.12-slim
WORKDIR /app
# Pinned per FR-10: the full reproducibility set is installed on every
# mock image so any mock can take on any role without a pip diff.
RUN pip install --no-cache-dir \
    hypercorn==0.17.3 \
    uvicorn==0.30.6 \
    starlette==0.38.6 \
    paramiko==3.4.0

COPY sidecar/tests/parity/fixtures/mock-example-http/server.py ./server.py

CMD ["python3", "server.py"]
