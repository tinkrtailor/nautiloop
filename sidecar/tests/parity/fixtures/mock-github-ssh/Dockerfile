# Mock github SSH server (FR-9). paramiko + threading HTTP
# introspection, all in one process.

FROM python:3.12-slim
WORKDIR /app
# Pinned per FR-10: the full reproducibility set is installed on every
# mock image so any mock can take on any role without a pip diff.
RUN pip install --no-cache-dir \
    hypercorn==0.17.3 \
    uvicorn==0.30.6 \
    starlette==0.38.6 \
    paramiko==3.4.0

COPY sidecar/tests/parity/fixtures/mock-github-ssh/server.py ./server.py
COPY sidecar/tests/parity/fixtures/mock-github-ssh/host_key ./host_key
COPY sidecar/tests/parity/fixtures/mock-github-ssh/host_key.pub ./host_key.pub
COPY sidecar/tests/parity/fixtures/mock-github-ssh/authorized_keys ./authorized_keys

ENV MOCK_HOST_KEY_PATH=/app/host_key

CMD ["python3", "server.py"]
