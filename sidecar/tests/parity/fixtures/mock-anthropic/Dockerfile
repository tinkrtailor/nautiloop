# Mock Anthropic HTTPS server (FR-8). Same shape as mock-openai.

FROM python:3.12-slim
WORKDIR /app
# Pinned per FR-10: the full reproducibility set is installed on every
# mock image so any mock can take on any role without a pip diff.
RUN pip install --no-cache-dir \
    hypercorn==0.17.3 \
    uvicorn==0.30.6 \
    starlette==0.38.6 \
    paramiko==3.4.0

COPY sidecar/tests/parity/fixtures/mock-anthropic/server.py ./server.py
COPY sidecar/tests/parity/fixtures/mock-anthropic/cert.pem ./cert.pem
COPY sidecar/tests/parity/fixtures/mock-anthropic/key.pem ./key.pem

ENV MOCK_CERT_PATH=/app/cert.pem
ENV MOCK_KEY_PATH=/app/key.pem

CMD ["python3", "server.py"]
