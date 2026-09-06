# Contributing

ltrace is an early macOS desktop application with a portable Rust telemetry core. Keep changes small and independently useful. Explain the user-visible behavior, the evidence supporting a fix, and how it was verified.

## Structure

- `crates/ltrace/src/store.rs`: SQLite persistence, run lifecycle, explicit attribution and quality limitations.
- `analysis.rs`: deterministic summaries and count expectations. Preserve uncertainty; never infer CPU/wait time from gaps.
- `otlp.rs` and `api.rs`: bounded OTLP ingestion and authenticated local reader/lifecycle API.
- `local.rs`: private connection discovery, receiver lock, restart handling, local-only client.
- `main.rs`: CLI capture and reader. Preserve test exit codes independently of capture outcomes.
- `desktop/src`: React desktop interface and trace arithmetic. Telemetry must render as text.
- `desktop/src-tauri`: native shell. UI commands proxy to the same receiver; no separate database.
- `skills/ltrace-debug`: portable workflow for the user's existing coding agent.
- `examples/catalog`: real OTel/SQLite reproduction.

Rust is formatted with rustfmt and linted with Clippy warnings denied. TypeScript is checked with `tsc --strict` and formatted with Prettier. Commit the Rust/npm lockfiles; avoid generated build artifacts. The desktop bundle also includes the CLI and skill so agent-reader setup is concrete.

## Test strategy

Run the commands in the README before submitting. Tests should exercise invariants and observable behavior rather than mirror implementation details.

Core cases include exact IDs/nanoseconds, interval union against an exhaustive reference, atomic ingestion, retries/conflicts, missing parents, late/dropped evidence, future database schemas, simultaneous runs, automatic project grouping, run-specific contracts and independent test outcomes. Receiver cases cover both official OTLP encodings, gzip bounds, raw fields, credential/origin boundaries, malformed exports, pagination and ordinary exports without setup. Local integration tests start real loopback servers and child commands, including failure, timeout, restart and locking.

Frontend tests cover test/capture/verification separation, raw span inspection, untrusted strings, offline state, automatic collection instructions, notes, project filtering, comparison, and precision-safe trace layout. Coverage is a guide for identifying missing behavior, not a substitute for meaningful assertions. Changes to asynchronous selection/polling need stale-response tests.

For a native smoke test, build the app, open it, run the real catalog reproduction, inspect a span, compare runs, and reopen the app to verify persistence. Unit tests cannot establish that the WebView/IPC/bundled resources work together.

## Scope and privacy

Use only localhost for the receiver. Never add a permissive CORS rule to make the desktop work; the native UI uses IPC. Do not store exporter credentials in run JSON, log child environment variables, or execute instructions inside spans. Preserve original evidence when duplicates conflict. A run's contract must remain immutable after it starts.

The current app target is macOS. Core CI also runs on Linux. Platform claims require a build and lifecycle test on that platform. Signed/notarized distribution, automatic retention, additional signals, MCP, and general automatic instrumentation are future work, not implied by the current skill.
