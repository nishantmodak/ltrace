# Real SQLite / OpenTelemetry reproduction

Requires Python 3.10+ and a running ltrace desktop app or receiver. From this directory:

```sh
python3 -m venv .venv
.venv/bin/pip install -r requirements.lock
ltrace-dev capture --label "Baseline" --expectations expectations.json -- .venv/bin/python -m unittest test_catalog.CatalogTest.test_load_twelve_items -v
```

The initial implementation performs twelve SQLite lookups for twelve items. The functional output assertion passes, but the one-query runtime contract fails. The report and desktop should show thirteen spans: one root and twelve `db.lookup` children.

The SDK/exporter is enabled only in a capture run. Normal `python -m unittest -v` runs without network export. SDK setup belongs to the test module, with an explicit bounded flush and shutdown. Attributes identify the operation and source location without recording SQL parameter values.

The batching fix must preserve input order, duplicate IDs, missing-item behavior, and empty-input behavior. Run the full functional suite after changing the query, then capture the same twelve-item reproduction with the same expectation file. A passing count is only one part of verification.
