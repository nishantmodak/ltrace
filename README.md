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

Each selected trace is checked for repeated SQL, slow dependencies, and retries. Findings appear above its waterfall, with the same JSON available to coding agents:

```sh
ltrace-dev findings RUN_ID TRACE_ID
```

The initial rules are deliberately explicit: at least five sibling database spans with identical recorded `db.query.text`/`db.statement`, SQL spans taking at least 100 ms or HTTP spans taking at least 250 ms, and sibling attempts with distinct `retry.attempt`/`http.request.resend_count` values and an observed failure. Query literals are not normalized. These are investigation leads, not proof that batching or changing retry behavior is correct. Missing attributes and capture problems are shown as detection limits; no findings is not a correctness guarantee.

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
