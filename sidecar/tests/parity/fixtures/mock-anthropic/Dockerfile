# Mock Anthropic HTTPS server (FR-8). Same shape as mock-openai.

FROM python:3.12-slim
WORKDIR /app
RUN pip install --no-cache-dir \
    hypercorn==0.17.3 \
    starlette==0.38.6

COPY sidecar/tests/parity/fixtures/mock-anthropic/server.py ./server.py
COPY sidecar/tests/parity/fixtures/mock-anthropic/cert.pem ./cert.pem
COPY sidecar/tests/parity/fixtures/mock-anthropic/key.pem ./key.pem

ENV MOCK_CERT_PATH=/app/cert.pem
ENV MOCK_KEY_PATH=/app/key.pem

CMD ["python3", "server.py"]
