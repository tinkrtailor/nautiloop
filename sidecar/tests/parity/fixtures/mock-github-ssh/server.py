#!/usr/bin/env python3
"""Mock github SSH server for the Nautiloop parity harness (FR-9).

Three ports, one process:

- 22/tcp   - paramiko ServerInterface accepting any client key.
             Recognizes `git-upload-pack <path>` and
             `git-receive-pack <path>` exec requests; rejects
             everything else with ExitStatus(128). env / pty-req /
             subsystem / shell / x11-req all return False.
- 2200/tcp - plain TCP listener used by the docker-compose
             healthcheck (accept + close).
- 9999/tcp - HTTP introspection API per FR-13.

Health probes on :2200 are NEVER logged to the introspection store.
"""
import base64
import json
import os
import socket
import sys
import threading
import time
import traceback
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import List, Optional

import paramiko


# ---------- introspection store ----------


@dataclass
class ObservedRequest:
    method: str
    path: str
    host_header: str
    headers: dict
    body_b64: str
    source_ip: str
    timestamp: str
    id: int


class Store:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._entries: List[ObservedRequest] = []
        self._next_id = 0

    def record(
        self,
        method: str,
        path: str,
        headers: dict,
        body: bytes,
        source_ip: str,
    ) -> None:
        with self._lock:
            self._next_id += 1
            self._entries.append(
                ObservedRequest(
                    method=method,
                    path=path,
                    host_header=headers.get("host", ""),
                    headers=headers,
                    body_b64=base64.b64encode(body).decode(),
                    source_ip=source_ip,
                    timestamp="",
                    id=self._next_id,
                )
            )

    def snapshot(self) -> List[dict]:
        with self._lock:
            return [e.__dict__ for e in self._entries]

    def reset(self) -> None:
        with self._lock:
            self._entries.clear()
            self._next_id = 0


STORE = Store()


# ---------- paramiko server ----------


ALLOWED_REPO_PATHS = {"test/repo.git", "'test/repo.git'"}
DETERMINISTIC_UPLOAD_PACK_BYTES = b"NAUTILOOP-PARITY-MOCK-PACK-v1\n"
DETERMINISTIC_RECEIVE_PACK_ACK = b"unpack ok\n"


def _strip_repo(arg: str) -> str:
    return arg.strip().strip("'").strip('"').lstrip("/")


def _parse_exec(command: bytes) -> Optional[tuple]:
    """Return (command_name, repo_path) or None if unrecognized."""
    try:
        s = command.decode()
    except UnicodeDecodeError:
        return None
    parts = s.split(" ", 1)
    if len(parts) == 2 and parts[0] in ("git-upload-pack", "git-receive-pack"):
        return parts[0], _strip_repo(parts[1])
    return None


class SSHServer(paramiko.ServerInterface):
    def __init__(self, source_ip: str):
        super().__init__()
        self.source_ip = source_ip
        self.event = threading.Event()
        self.parsed_exec: Optional[tuple] = None
        self.exec_raw: bytes = b""
        self.stdin_bytes: bytes = b""

    # FR-9: accepts any client key.
    def check_auth_publickey(self, username, key):
        return paramiko.AUTH_SUCCESSFUL

    def check_auth_none(self, username):
        return paramiko.AUTH_SUCCESSFUL

    def get_allowed_auths(self, username):
        return "publickey,none"

    def check_channel_request(self, kind, chanid):
        if kind == "session":
            return paramiko.OPEN_SUCCEEDED
        return paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_exec_request(self, channel, command):
        self.exec_raw = command
        self.parsed_exec = _parse_exec(command)
        self.event.set()
        return True

    # FR-9 rejections: return False for env/pty-req/shell/x11/subsystem.
    def check_channel_env_request(self, channel, name, value):
        return False

    def check_channel_pty_request(
        self, channel, term, width, height, pixelwidth, pixelheight, modes
    ):
        return False

    def check_channel_shell_request(self, channel):
        return False

    def check_channel_subsystem_request(self, channel, name):
        return False

    def check_channel_x11_request(
        self, channel, single_connection, auth_protocol, auth_cookie, screen_number
    ):
        return False


def handle_connection(client: socket.socket, addr) -> None:
    host_key_path = os.environ.get("MOCK_HOST_KEY_PATH", "/app/host_key")
    try:
        host_key = paramiko.Ed25519Key(filename=host_key_path)
    except Exception as e:
        print(f"host key load failed: {e}", file=sys.stderr)
        client.close()
        return

    source_ip, _ = addr
    transport = paramiko.Transport(client)
    transport.add_server_key(host_key)
    server = SSHServer(source_ip)
    try:
        transport.start_server(server=server)
    except Exception as e:
        print(f"SSH handshake failed from {source_ip}: {e}", file=sys.stderr)
        transport.close()
        return

    channel = transport.accept(20)
    if channel is None:
        transport.close()
        return
    if not server.event.wait(10):
        # No exec received within 10s — close.
        channel.close()
        transport.close()
        return

    parsed = server.parsed_exec
    exec_raw = server.exec_raw.decode(errors="replace")
    if parsed is None:
        STORE.record(
            method="EXEC",
            path=exec_raw,
            headers={"host": "mock-github-ssh"},
            body=b"",
            source_ip=source_ip,
        )
        channel.send_stderr(b"unrecognized exec command\n")
        channel.send_exit_status(128)
        channel.close()
        transport.close()
        return

    cmd, repo = parsed
    # FR-9: only test/repo.git is allowed. Wrong path -> 128.
    if repo != "test/repo.git":
        STORE.record(
            method="EXEC",
            path=exec_raw,
            headers={"host": "mock-github-ssh"},
            body=b"",
            source_ip=source_ip,
        )
        channel.send_stderr(b"fatal: repository not found\n")
        channel.send_exit_status(128)
        channel.close()
        transport.close()
        return

    if cmd == "git-upload-pack":
        # Fetch path: write deterministic pack bytes.
        STORE.record(
            method="EXEC",
            path=exec_raw,
            headers={"host": "mock-github-ssh"},
            body=b"",
            source_ip=source_ip,
        )
        channel.sendall(DETERMINISTIC_UPLOAD_PACK_BYTES)
        channel.send_exit_status(0)
    else:  # git-receive-pack
        # Push path: consume incoming bytes from stdin, then ack.
        try:
            channel.settimeout(5.0)
            incoming = b""
            while True:
                chunk = channel.recv(8192)
                if not chunk:
                    break
                incoming += chunk
                if len(incoming) > 16 * 1024:
                    break
            server.stdin_bytes = incoming
        except Exception:
            pass
        STORE.record(
            method="EXEC",
            path=exec_raw,
            headers={"host": "mock-github-ssh"},
            body=server.stdin_bytes,
            source_ip=source_ip,
        )
        channel.sendall(DETERMINISTIC_RECEIVE_PACK_ACK)
        channel.send_exit_status(0)

    channel.close()
    transport.close()


def ssh_server_loop() -> None:
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(("0.0.0.0", 22))
    sock.listen(8)
    print("mock-github-ssh: listening on :22", file=sys.stderr)
    while True:
        client, addr = sock.accept()
        t = threading.Thread(target=handle_connection, args=(client, addr), daemon=True)
        t.start()


def tcp_health_loop() -> None:
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(("0.0.0.0", 2200))
    sock.listen(8)
    print("mock-github-ssh: health listener on :2200", file=sys.stderr)
    while True:
        client, _ = sock.accept()
        # FR-13: health probes are NEVER logged.
        try:
            client.close()
        except Exception:
            pass


# ---------- HTTP introspection on :9999 ----------


class IntrospectHandler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        # Silence stdlib access log — we don't want it polluting
        # container stderr.
        pass

    def _write_json(self, status: int, payload) -> None:
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/__harness/logs":
            self._write_json(200, STORE.snapshot())
        else:
            self._write_json(404, {"error": "not found"})

    def do_POST(self):
        if self.path == "/__harness/reset":
            STORE.reset()
            self._write_json(200, {"ok": True})
        else:
            self._write_json(404, {"error": "not found"})


def introspection_loop() -> None:
    httpd = ThreadingHTTPServer(("0.0.0.0", 9999), IntrospectHandler)
    print("mock-github-ssh: introspection on :9999", file=sys.stderr)
    httpd.serve_forever()


def main() -> None:
    threads = [
        threading.Thread(target=tcp_health_loop, daemon=True),
        threading.Thread(target=introspection_loop, daemon=True),
    ]
    for t in threads:
        t.start()
    try:
        ssh_server_loop()
    except KeyboardInterrupt:
        sys.exit(0)
    except Exception:
        traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
