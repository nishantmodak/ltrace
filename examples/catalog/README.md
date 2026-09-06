# Real SQLite / OpenTelemetry reproduction

This example preserves output correctness while replacing twelve per-item SQLite lookups with one batch query. It uses the official Python OTel SDK and actual SQLite work, not generated trace fixtures.

From the repository root, with Python 3.10+:

```sh
python3 -m venv examples/catalog/.venv
examples/catalog/.venv/bin/pip install -r examples/catalog/requirements.lock
cargo build --locked -p ltrace-local --bin ltrace-dev
examples/catalog/.venv/bin/python scripts/verify_workflow.py
```

The verification script creates a temporary reproduction and receiver, captures `fixtures/baseline_catalog.py`, applies the current `catalog.py`, reruns the same test and contract, checks raw evidence through the CLI, sends ordinary SDK exports with no run setup, and restarts the receiver to verify persistence. It does not modify the example's source files.

Expected evidence:

| Reproduction | Output test | Query spans | Runtime contract |
| --- | --- | --- | --- |
| Baseline | Passed | 12 | Failed |
| Batched implementation | Passed | 1 | Passed |

Each trace also contains one `catalog.load_items` root span. The full functional suite verifies ordering, duplicate IDs, missing items, and empty input after the fix. This example exercises a twelve-item workload, not arbitrary database limits or statistically reliable latency improvements.

## Watch in the desktop

Open ltrace and use its displayed data directory:

```sh
examples/catalog/.venv/bin/python scripts/verify_workflow.py --home '/path/from/connection/details'
```

Both captures and a separate Incoming traces stream appear in the desktop. Select a request to inspect its waterfall and attributes. **Run details** contains the test result, expectation and notes; choose the baseline there to compare operation counts.

For an individual test from this example directory:

```sh
ltrace-dev capture --label "Batch lookup" --expectations expectations.json -- .venv/bin/python -m unittest test_catalog.CatalogTest.test_load_twelve_items -v
```

The SDK/exporter is test-local and enabled only for a capture run or an explicitly configured trace endpoint. Normal tests create no exporter. The provider has a bounded flush and shutdown. Attributes identify the operation and source without recording SQL parameter values.
