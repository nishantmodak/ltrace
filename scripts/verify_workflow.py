#!/usr/bin/env python3
"""Exercise the real SDK, CLI, receiver, reader, persistence and batching fix.

Run with the catalog virtual environment. No external services are contacted.
--home attaches to an already running desktop so the same evidence is visible.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, default=ROOT / "target/debug/ltrace-dev")
    parser.add_argument("--home", type=Path, help="Use an existing receiver")
    parser.add_argument("--scratch-parent", type=Path, help="Place the temporary reproduction inside a project")
    args = parser.parse_args()
    cli = args.cli.resolve()
    if args.scratch_parent:
        args.scratch_parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="ltrace-example-", dir=args.scratch_parent) as temporary:
        base = Path(temporary)
        project = base / "catalog"
        shutil.copytree(ROOT / "examples/catalog", project, ignore=shutil.ignore_patterns(".venv", "__pycache__"))
        home = (args.home or base / "data").resolve()
        env = dict(os.environ, LTRACE_HOME=str(home), PYTHONDONTWRITEBYTECODE="1")
        server = None

        def command(*argv, check=True):
            result = subprocess.run([str(cli), *argv], cwd=project, env=env, text=True, capture_output=True, timeout=30)
            if check and result.returncode:
                raise AssertionError(f"Command failed ({result.returncode}): {result.stderr}")
            return json.loads(result.stdout)

        def start():
            process = subprocess.Popen([str(cli), "--home", str(home), "serve", "--port", "0"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise AssertionError("Receiver exited during startup")
                if (home / "connection.json").exists():
                    try:
                        command("doctor")
                        return process
                    except (AssertionError, json.JSONDecodeError):
                        pass
                time.sleep(0.05)
            process.kill()
            process.wait()
            raise AssertionError("Receiver startup timed out")

        def stop(process):
            process.send_signal(signal.SIGINT)
            process.wait(timeout=10)

        try:
            if not args.home:
                server = start()
            command("doctor")
            results = []
            for label, source, expected, verdict in [
                ("Baseline · repeated queries", ROOT / "examples/catalog/fixtures/baseline_catalog.py", 12, "failed"),
                ("After fix · batch query", ROOT / "examples/catalog/catalog.py", 1, "passed"),
            ]:
                shutil.copy2(source, project / "catalog.py")
                report = command("capture", "--label", label, "--command-label", "Load twelve items", "--expectations", "expectations.json", "--", sys.executable, "-m", "unittest", "test_catalog.CatalogTest.test_load_twelve_items", "-v")
                assert report["run"]["test_status"] == "passed", report
                assert report["run"]["capture_status"] == "settled", report
                assert report["verification"][0]["observed_count"] == expected, report
                assert report["verification"][0]["status"] == verdict, report
                assert report["span_count"] == expected + 1, report
                rid = report["run"]["id"]
                index = command("traces", rid)
                trace = index["traces"][0]
                evidence = command("span", rid, trace["trace_id"], trace["root_span_id"])
                assert evidence["span"]["name"] == "catalog.load_items", evidence
                assert evidence["span"]["start_ns"] == trace["start_ns"], evidence
                command("note", report["run"]["session_id"], "--run", rid, "--body", f"Real SQLite reproduction: {expected} lookup spans; output assertion passed. Runtime contract {verdict}.")
                results.append({"run_id": rid, "queries": expected, "test": "passed", "expectation": verdict})

            # Full business behavior remains checked after the runtime improvement.
            subprocess.run([sys.executable, "-m", "unittest", "-v"], cwd=project, check=True, timeout=20, env={k:v for k,v in env.items() if not k.startswith("OTEL_") and k != "LTRACE_RUN_ID"}, stdout=subprocess.DEVNULL)

            # Plain SDK exports must appear without creating a session or forging test attribution.
            info = json.loads((home / "connection.json").read_text())
            direct_env = {k:v for k,v in env.items() if not k.startswith("OTEL_") and k != "LTRACE_RUN_ID"}
            direct_env.update(OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=info["url"] + "/v1/traces", OTEL_EXPORTER_OTLP_TRACES_PROTOCOL="http/protobuf")
            subprocess.run([sys.executable, "-c", "from test_catalog import setUpModule, tearDownModule, CatalogTest; setUpModule(); case=CatalogTest('test_load_twelve_items'); case.setUp(); case.test_load_twelve_items(); case.doCleanups(); tearDownModule()"], cwd=project, env=direct_env, check=True, timeout=20)
            incoming = next(p for p in command("sessions")["sessions"] if p["project"] == "ltrace:incoming")
            stream = command("show", "session", incoming["id"])["runs"][0]
            assert stream["test_status"] == "not_run", stream
            assert command("show", "run", stream["id"])["span_count"] >= 2

            if server:
                stop(server)
                server = start()
                for result in results:
                    restored = command("show", "run", result["run_id"])
                    assert restored["verification"][0]["status"] == result["expectation"]
            print(json.dumps({"results":results,"direct_exports":"visible without setup","persistence":"verified" if server else "using desktop store"}, indent=2))
        finally:
            if server and server.poll() is None:
                stop(server)


if __name__ == "__main__":
    main()
