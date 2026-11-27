"""End-to-end verification runner for the Arcnix Redis app.

1. Starts a local loopback SSH/SFTP test server (`tools/sftp_fixture.py`) with
   live stateful RESP2 Redis instances (host + password-protected Docker).
2. Runs all 12 core SSH integration test cases (`arcnix-core --test redis_integration`).
3. Runs the live full-stack Arcnix UI + SSH worker E2E test (`redis_ui::tests::live_ssh_ui_e2e_flow`).
4. Builds and launches the real compiled `arcnix` desktop binary connected to the
   local test server, verifies startup stability and real SSH traffic, and checks
   that no Redis password ever appeared in SSH command lines.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def eventually(probe, timeout=15):
    end = time.monotonic() + timeout
    last = None
    while time.monotonic() < end:
        try:
            val = probe()
            if val:
                return val
        except Exception as exc:
            last = exc
        time.sleep(0.1)
    raise AssertionError(f"Timed out waiting for fixture condition: {last}")


def write_test_config(run_dir: Path, info: dict) -> Path:
    host, port_str = info["address"].rsplit(":", 1)
    local_dir = run_dir / "local"
    local_dir.mkdir(exist_ok=True)
    app_config = {
        "version": 1,
        "config": {
            "servers": [
                {
                    "id": "redis-test-server",
                    "name": "Redis Test Server",
                    "host": host,
                    "port": int(port_str),
                    "username": "arcnix",
                    "auth": "PrivateKey",
                    "private_key": info["client_key_ed25519"],
                    "initial_remote_path": "/",
                }
            ],
            "groups": [],
            "trusted_hosts": [
                {
                    "host": host,
                    "port": int(port_str),
                    "fingerprint": info["host_key_rsa_fingerprint"],
                }
            ],
            "local_directory": str(local_dir),
            "settings": {
                "language": "English",
                "theme": "Light",
                "show_hidden_files": True,
                "resource_interval_seconds": 3,
            },
        },
    }
    config_path = run_dir / "config.json"
    config_path.write_text(json.dumps(app_config), encoding="utf-8")
    return config_path


def find_binary() -> Path:
    meta = json.loads(
        subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"], cwd=ROOT)
    )
    return Path(meta["target_directory"]) / "debug" / ("arcnix.exe" if os.name == "nt" else "arcnix")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--preview",
        action="store_true",
        help="Start the local SSH + Redis test server and keep Arcnix open for interactive manual E2E testing.",
    )
    args = parser.parse_args()

    run_dir = Path(tempfile.mkdtemp(prefix="arcnix-redis-e2e-"))
    ready_file = run_dir / "fixture-ready.json"
    peer_log = (run_dir / "peer.log").open("w", encoding="utf-8")
    peer = subprocess.Popen(
        [sys.executable, str(ROOT / "tools/sftp_fixture.py"), "--ready", str(ready_file)],
        cwd=ROOT,
        stdout=peer_log,
        stderr=peer_log,
    )
    try:
        info = eventually(lambda: json.loads(ready_file.read_text(encoding="utf-8")))
        fixture_root = Path(info["root"])
        (fixture_root / "redis-enabled").write_text("1\n", encoding="utf-8")
        addr = info["address"]
        print(f"Local SSH + Redis test server listening at {addr} (root: {fixture_root})", flush=True)

        if args.preview:
            binary = find_binary()
            if not binary.is_file():
                subprocess.run(["cargo", "build", "--locked", "-p", "arcnix", "--bin", "arcnix"], cwd=ROOT, check=True)
            config_path = write_test_config(run_dir, info)
            print("Interactive Redis E2E preview ready:", flush=True)
            print("  - Server: Redis Test Server (one-click connect, Ed25519 key pre-configured)", flush=True)
            print("  - Host Redis (This server): Redis 7.2.5, no password, db0 (123 keys) & db1 (2 keys)", flush=True)
            print("  - Docker Redis (redis-auth): Redis 7.4.1, password = s3cr3t", flush=True)
            print("Close the Arcnix window when finished to shut down the local test server.", flush=True)
            proc = subprocess.Popen([str(binary), "--config", str(config_path)], cwd=ROOT)
            return proc.wait()

        env = os.environ.copy()
        env["ARCNIX_TEST_SSH_ADDR"] = addr
        env["ARCNIX_TEST_SSH_KEY"] = info["client_key_ed25519"]

        print("\n=== Suite 1: arcnix-core SSH + RESP2 integration tests (TC-01..TC-12) ===", flush=True)
        subprocess.run(
            [
                "cargo",
                "test",
                "--locked",
                "-p",
                "arcnix-core",
                "--test",
                "redis_integration",
                "--",
                "--ignored",
                "--nocapture",
            ],
            cwd=ROOT,
            env=env,
            check=True,
        )

        print("\n=== Suite 2: arcnix live UI + SSH worker E2E flow ===", flush=True)
        subprocess.run(
            [
                "cargo",
                "test",
                "--locked",
                "-p",
                "arcnix",
                "software::redis_ui::tests::live_ssh_ui_e2e_flow",
                "--",
                "--ignored",
                "--nocapture",
            ],
            cwd=ROOT,
            env=env,
            check=True,
        )

        print("\n=== Suite 3: Build & launch real Arcnix desktop binary against local test server ===", flush=True)
        subprocess.run(["cargo", "build", "--locked", "-p", "arcnix", "--bin", "arcnix"], cwd=ROOT, check=True)
        binary = find_binary()
        assert binary.is_file(), f"Missing built binary: {binary}"

        config_path = write_test_config(run_dir, info)
        app_log_path = run_dir / "arcnix-app.log"
        with app_log_path.open("w", encoding="utf-8") as app_log:
            proc = subprocess.Popen(
                [str(binary), "--config", str(config_path)],
                cwd=ROOT,
                stdout=app_log,
                stderr=app_log,
            )
            try:
                time.sleep(3.5)
                rc = proc.poll()
                assert rc is None, f"Arcnix exited unexpectedly with code {rc}: {app_log_path.read_text()}"
                print(f"[PASS] Real Arcnix desktop binary launched and remained healthy (PID {proc.pid})", flush=True)
            finally:
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=5)

        # Security audit: confirm no Redis password ever appeared in SSH command lines.
        queries_path = fixture_root / "server-app-queries.jsonl"
        assert queries_path.is_file(), "Expected server-app-queries.jsonl audit log"
        queries_text = queries_path.read_text(encoding="utf-8")
        assert "# arcnix:apps:discover" in queries_text
        assert "# arcnix:redis:helper" in queries_text
        assert "s3cr3t" not in queries_text, "SECURITY FAILURE: Redis password leaked into SSH command line!"
        assert "wrong-pass" not in queries_text, "SECURITY FAILURE: Redis password leaked into SSH command line!"
        print("[PASS] Security audit: zero Redis credentials in SSH command audit log", flush=True)
        print("\nALL REDIS E2E VERIFICATION SUITES PASSED.", flush=True)
        return 0
    finally:
        peer.terminate()
        try:
            peer.wait(timeout=10)
        except subprocess.TimeoutExpired:
            peer.kill()
            peer.wait(timeout=5)
        peer_log.close()


if __name__ == "__main__":
    raise SystemExit(main())
