# Local API v1

Run `cargo run -p ltrace-local --bin ltrace-dev -- serve`. Default port: 4318, bound only to `127.0.0.1`. Use `--port 0` for an available port. `ltrace-dev --help` identifies this product, avoiding confusion with Linux's unrelated `ltrace` binary.

The desktop and CLI discover `connection.json` in the application data directory (`LTRACE_HOME` overrides it). The directory is private and the connection file is mode 0600 on Unix. Management requests require its bearer token. Browser origins are rejected; the native UI uses IPC. Tokens are local capabilities; do not copy them into source control or agent messages. Proxy routing and redirects are disabled in the client.

## Operations

All management paths begin `/api/` and use JSON:

| Method | Path | Body / response |
| --- | --- | --- |
| GET | health | Product, version, API version |
| GET | sessions | Latest 200 sessions |
| POST | sessions | `{title, project, expectations: []}` |
| GET | sessions/:id | Session, latest 200 runs and notes |
| POST | sessions/:id/runs | `{label, command, revision}`; returns run and private export token |
| POST | sessions/:id/notes | `{run_id: null, body}` |
| POST | runs/:id/finish | `{exit_code: 0, issue: null}`; can finish once |
| GET | runs/:id | Test/capture states, quality issues, first 100 operation groups, expectations |
| GET | runs/:id/spans | Raw spans, `total`, `next_offset`; `offset`, `limit` (1–200), optional `trace_id` |
| GET | runs/:id/spans/:trace/:span | Raw span and uncovered recorded child time |

Expectation shape: `{name, reason, service, operation, min_count, max_count}`. Names match exactly. Expectations are immutable within a session and snapshotted into runs. Create a new session for a changed contract. Each verification cites up to 20 span references. A missing matching operation stays unknown, including expectations of zero executions. Counts exceeding the maximum are observed counterexamples even with partial data. Tests remain the authority for functional correctness.

## Export and limits

`POST /v1/traces` accepts OTLP HTTP `application/x-protobuf` and `application/json`, with identity or gzip encoding. Each export requires `x-ltrace-run-token`, supplied automatically by `ltrace-dev capture` through `OTEL_EXPORTER_OTLP_TRACES_HEADERS`. No arrival-time attribution or default run is used. gRPC, metrics, and log ingestion are not supported in v0.

Limits: 4 MiB compressed and expanded requests, 10,000 spans per request, 64 KiB per span including its resource/scope, and 50,000 spans or 64 MiB of stored span JSON per run. Oversized/malformed exports taint the attributed run. Storage-limit rejections return OTLP partial success. Exact retries are idempotent. Conflicting IDs preserve the first version and taint the run. Exports received after finishing are retained with a partial-capture warning.

SDK shutdown/flush is required before the test exits. The capture command adds a bounded grace period, not a completeness guarantee. `settled` means the capture window closed with spans and no recorded ingestion issue; sampling, missing instrumentation, or unseen services can still create gaps. The runner sets `always_on` sampling for the child process, but SDK code may override it.

A receiver lock prevents two processes serving the same store. After a restart, unfinished runs become interrupted/partial. SQLite uses WAL and refuses a future schema version. History is local and currently retained until the data directory is removed while the receiver is stopped. There is no automatic retention policy yet.

## Command results

`capture` sends child output to stderr and emits the JSON run summary on stdout. Its exit code preserves the test exit code; interrupted/spawn-failed tests or collector failure after a successful test return 2. Runtime expectation failures are reported in JSON and do not replace the test result. It stores a safe command description, not full arguments, environment variables, or output. Use `--command-label` for a meaningful non-sensitive description.
