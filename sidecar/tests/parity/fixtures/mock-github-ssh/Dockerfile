# Mock github SSH server (FR-9). paramiko + threading HTTP
# introspection, all in one process.

FROM python:3.12-slim
WORKDIR /app
RUN pip install --no-cache-dir \
    paramiko==3.4.0

COPY sidecar/tests/parity/fixtures/mock-github-ssh/server.py ./server.py
COPY sidecar/tests/parity/fixtures/mock-github-ssh/host_key ./host_key
COPY sidecar/tests/parity/fixtures/mock-github-ssh/host_key.pub ./host_key.pub
COPY sidecar/tests/parity/fixtures/mock-github-ssh/authorized_keys ./authorized_keys

ENV MOCK_HOST_KEY_PATH=/app/host_key

CMD ["python3", "server.py"]
