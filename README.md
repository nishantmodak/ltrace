# ltrace

Local runtime evidence for developers and their coding agents. Open the desktop app, run your instrumented code, and watch traces arrive. There is no debugging-session setup step and no additional AI model to configure.

**Working macOS preview:** Rust receiver and SQLite store, Tauri desktop, bundled CLI reader/capture runner, and a companion agent skill. The desktop and CLI read the same local evidence. This is an unsigned development build, not a published release.

## Run the desktop

Prerequisites: Rust 1.95+, Node.js 22+, and the [Tauri native prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS. The desktop bundle is currently targeted and locally verified on macOS; other desktop platforms are not yet release-tested.

```sh
npm ci --prefix desktop
npm run desktop --prefix desktop
```

For a standalone app:

```sh
npm run desktop:build --prefix desktop
```

The macOS bundle is `target/release/bundle/macos/ltrace.app`. It includes `ltrace-dev` and the skill in its Resources directory. **Connection details** in the app provides the exact capture command and skill location. You can also install the CLI on your PATH:

```sh
cargo install --locked --path crates/ltrace
```

For CLI-only use, start `ltrace-dev serve`. The desktop attaches to an existing compatible receiver or starts one automatically. The default receiver is `http://127.0.0.1:4318`. Port conflicts are reported; `LTRACE_PORT` configures the desktop port and `serve --port` configures the standalone receiver.

## Traces just show up

Point an existing OTel SDK/exporter at the receiver:

```sh
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://127.0.0.1:4318/v1/traces
OTEL_EXPORTER_OTLP_TRACES_PROTOCOL=http/protobuf
```

All recent traces appear together, newest first, without project or run dropdowns. Search by operation, service, or trace ID. Ordinary exports are not attributed to a test. The app shows operations, trace waterfalls, exact IDs/timestamps, attributes, events, links, and instrumentation resources. It accepts OTLP HTTP protobuf or JSON, with gzip support.

The application still needs instrumentation and an SDK flush before short-lived processes exit. For code without tracing, the companion skill guides your existing coding agent to add focused OTel instrumentation.

## Let your coding agent inspect a test

From the application repository:

```sh
ltrace-dev capture --label "Reproduce lookup issue" -- your-test-command
```

The runner automatically identifies the project, starts a run, supplies isolated local trace export settings to the child process, records its result, and prints a structured evidence report. Child output goes to stderr. Project/run grouping happens automatically; simultaneous captures receive separate export credentials.

Optional runtime expectations attach to the run:

```sh
ltrace-dev capture --expectations expectations.json -- your-test-command
ltrace-dev show run RUN_ID
ltrace-dev traces RUN_ID --search catalog
ltrace-dev spans RUN_ID --limit 100
ltrace-dev span RUN_ID TRACE_ID SPAN_ID
ltrace-dev note PROJECT_ID --run RUN_ID --body "Observed twelve lookup spans; inspecting the loop."
```

The report contains the project grouping ID as `run.session_id`. The `session` name in storage/API fields is an internal grouping detail. Normal capture requires no session ID or creation command.

An expectation specifies an exact service/operation, count range, and source-backed reason. See [the example contract](examples/catalog/expectations.json). Each run retains its own contract. The desktop separates **functional test result**, **capture health**, and **runtime verification**. A missing or partial capture cannot silently become a pass.

Give your coding agent [the companion skill](skills/ltrace-debug/SKILL.md). It describes how to instrument, run, inspect, fix, and verify using the implemented commands. Skill loading depends on the coding agent's configuration; ltrace does not claim to force an agent to inspect evidence.

## Automatic findings

**The app does not use AI to generate findings.** Detection runs locally as deterministic Rust rules; explanations and suggestions are predefined text. There are no model calls, AI-provider API keys, or cloud uploads. Your existing coding agent can read the same evidence, inspect the source, propose a fix, and rerun tests through the companion skill.

Findings appear **above the selected trace's waterfall**. Clicking one expands branches, highlights matching spans, and opens its explanation and evidence on the right. The CLI returns the same report:

```sh
ltrace-dev findings RUN_ID TRACE_ID
```

These are all implemented patterns:

| Pattern | Exact trigger | Interpretation and limitations |
| --- | --- | --- |
| Possible N+1 | **5 or more spans** with the same trace, service, nonempty parent ID, and recorded query text from `db.query.text` or `db.statement`. | Only surrounding whitespace is trimmed. SQL literals, case, and internal whitespace are not normalized. Repetition can be intentional; inspect source and workload before batching. |
| Slow SQL | A database span takes **at least 100 ms**. Database identity comes from query text, `db.system.name`, or `db.system`. | Fixed starting threshold, not an application-specific budget. Requires valid timestamps. |
| Slow HTTP | An HTTP span takes **at least 250 ms**. Identity comes from `http.request.method`, `http.method`, `http.response.status_code`, `http.status_code`, `url.full`, or `http.url`. | Fixed starting threshold. Names such as `GET /items` alone do not establish HTTP instrumentation. |
| Retry after failure | Sibling spans in the same trace and service contain **at least two distinct numeric attempt counters**, with at least one span marked as an error. Uses `retry.attempt` or `http.request.resend_count`. | Requires explicit metadata and a nonempty parent ID. Equal counters do not imply a retry. A successful recovery may need no code change. |

Slow SQL and HTTP are grouped under **Slow dependencies**. Counts refer to recorded spans, not unique network requests; overlapping durations are not summed into an estimated cost. Thresholds are currently fixed in the detector and are not configurable in the UI or CLI.

Missing query text, dropped telemetry, invalid timestamps, missing parents, and capture-quality problems appear as **Detection limits**. Observed findings remain useful in a partial capture, but an empty findings list does not prove correct behavior or complete instrumentation. Results are capped at 50 findings and 200 span references per finding, with truncation disclosed; the UI offers the first 20 evidence links while retaining the observed count.

Not implemented: separate redundant-call detection, HTTP N+1, excessive fanout, serialized-call analysis, pool saturation, or automatic cross-run performance regression detection. Explicit test expectations are a separate verification mechanism.

### Detector test cases

Run the focused suite without desktop dependencies:

```sh
cargo test -p ltrace-local --test findings --locked
```

The [detector tests](crates/ltrace/tests/findings.rs) cover:

- A checkout fixture with repeated SQL, slow SQL/HTTP, and retry recovery, plus a small health trace without findings.
- Four versus five repeated queries; current/legacy attributes; query text, parent, service, and trace boundaries; blank text and root spans.
- SQL and HTTP duration boundaries, unsupported operation names, invalid timing, and every supported HTTP identity attribute.
- Retry counters in integer/string form, both naming conventions, equal and malformed counters, missing failure markers, and unrelated requests.
- Partial telemetry, bounded evidence, deterministic output, the 50-finding limit, and API isolation when different runs reuse a trace ID.

Frontend tests additionally verify that selecting a finding highlights its spans and opens the cited raw evidence. These tests establish the implemented rules; they do not measure real-world precision or recall.

## Inspect small and complex demo traces

With the desktop open, run `python3 scripts/demo_traces.py`. This sends two explicitly **synthetic** examples through the local OTLP receiver:

- **Demo · Health check** — 3 spans, 24 ms, one service.
- **Demo · Checkout** — 30 spans, 1.2 s, four services, parallel inventory/cart/shipping work, twelve repeated lookups, and a payment timeout followed by a successful retry.

Select either request in the left list. Expand/collapse branches, click a timeline bar for its attributes, and inspect the failed payment span's exception event. Both carry `demo.synthetic=true`; durations are designed fixtures, not performance measurements. Select Checkout to see automatic findings above its waterfall: possible N+1 (12 queries), slow dependencies (2 spans), and retry after failure (2 attempts). Click a finding to highlight evidence and inspect its explanation. Use `--port` for a non-default local receiver.

## Try a real reproduction

[The catalog example](examples/catalog/README.md) uses Python's OTel SDK and actual SQLite queries. Its output tests stay independent of the runtime expectation. It demonstrates a repeated-query baseline followed by one batched query while preserving ordering, duplicate IDs, and missing-item behavior.

## Development and tests

```sh
npm ci --prefix desktop
npm run prepare:bundle --prefix desktop
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
npm run format:check --prefix desktop
npm run test:coverage --prefix desktop
```

Native tests require the platform prerequisites and frontend/bundled CLI assets, prepared above. Core-only development can use `cargo test -p ltrace-local --locked`. Integration tests bind ephemeral loopback sockets and test real child commands. See [CONTRIBUTING.md](CONTRIBUTING.md) for the test strategy and [local-api.md](docs/local-api.md) for the interface and limits.

## Boundaries of this preview

- Trace ingestion only: no OTLP gRPC, metrics, standalone log ingestion, or MCP server. The CLI is the agent reader.
- Verification supports exact operation counts. Repetition is evidence to inspect, not an automatic N+1 diagnosis. Functional tests establish output correctness.
- A closed capture window does not prove full instrumentation coverage. Sampling, SDK overrides, missing services, clock differences, and asynchronous work can limit conclusions.
- Captures preserve the first conflicting span and visibly flag rejected/late data. Raw telemetry is untrusted application data.
- Read APIs require a private local credential and reject browser origins. Plain OTLP ingestion accepts local exporters. The app makes no cloud uploads or model calls.
- Per-export and per-run storage is bounded. History has no automatic retention policy yet. `LTRACE_HOME` selects a separate local data directory.



MIT licensed.
