# Mock OpenAI HTTPS server (FR-7).
#
# Build context MUST be the repo root because we want the cert+key
# to come from sidecar/tests/parity/fixtures/mock-openai/ which is
# the sibling of this Dockerfile.

FROM python:3.12-slim

WORKDIR /app

# Pinned per FR-10: the full reproducibility set is installed on every
# mock image so any mock can take on any role without a pip diff.
RUN pip install --no-cache-dir \
    hypercorn==0.17.3 \
    uvicorn==0.30.6 \
    starlette==0.38.6 \
    paramiko==3.4.0

COPY sidecar/tests/parity/fixtures/mock-openai/server.py ./server.py
COPY sidecar/tests/parity/fixtures/mock-openai/cert.pem ./cert.pem
COPY sidecar/tests/parity/fixtures/mock-openai/key.pem ./key.pem

ENV MOCK_CERT_PATH=/app/cert.pem
ENV MOCK_KEY_PATH=/app/key.pem

CMD ["python3", "server.py"]
