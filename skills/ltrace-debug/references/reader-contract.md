# Proposed session and reader contract

Status: design only. No operation below is a callable command or MCP tool. Replace conceptual mappings with versioned, implemented interfaces before publishing executable examples.

## Design goals

The human viewer and agent reader should resolve the same session, trace, and span IDs. Return compact structured summaries with evidence references and allow selective expansion. Retrieval should not start a process, modify application code, replay a request, or forward telemetry.

The agent supplies instrumentation; the receiver does not insert it into the application. Agent and desktop UI share the same local data store. A debugging session contains the task, expectations, and multiple reproduction runs. Each run has its own repository state, command result, and telemetry attribution.

## Conceptual reader operations

| Operation | Input | Required result |
| --- | --- | --- |
| List sessions | Project filter, optional time window, page size/cursor | Matching session IDs and reproduction summaries; explicit pagination. |
| Read session | Session ID | Reproduction result, capture state, observed services, trace count, attribution basis, and data-quality limitations. |
| Find traces | Session ID, supported filters, page size/cursor | Stable trace IDs, root operation, duration, span count, error indicators, and partial-capture information. |
| Inspect trace | Session and trace ID, optional subtree/filter and pagination | Requested spans with original attributes/events/links, trace health, and references from summaries to source spans. |
| Summarize operations | Session/trace scope and grouping | Repeated operations, error locations, recorded interval coverage, evidence IDs, grouping rules, and skipped/unsupported records. |
| Read related logs | Session and trace/span ID, page size/cursor | Correlated records and basis for correlation; absence stated without inferring no logs were emitted. |
| Compare runs | Explicit baseline/candidate run IDs and scope | Functional results, expectation results, observed count/duration differences, matching basis, unmatched operations, and comparability limitations. |

The initial end-to-end slice requires session/run lookup, trace inspection, a small useful summary, and verification between two runs. Related logs can follow when supported by the selected stack. Expose operations only when implemented. Use the smallest useful result limit and include a cursor or truncation indicator rather than silently omitting evidence. Preserve trace IDs and nanosecond timestamps losslessly in structured output.

## Separate session lifecycle operations

These proposed mutations are distinct from the read-only reader. They write local session records; they do not execute application code:

- Open a debugging session with project identity, task, and source-backed expectations.
- Register a reproduction run and complete it with the real command result, code identity, capture state, and evidence attribution.
- Append instrumentation changes, observed findings, and concise agent actions linked to run/evidence IDs.
- Record verification per expectation: passed, failed, or unknown, including evidence IDs and limitations.

Identify whether a result came from a deterministic evaluator, a functional test, or the agent's interpretation. Preserve expectation revisions and their reasons; changing an expectation must not silently turn an old failure into a pass. Validate referenced IDs against the correct project/run.

The desktop timeline should update from these records and live telemetry arrivals. Append actions idempotently so retried writes do not duplicate activity. Record the original reproduction output or a local artifact reference, with redaction. Agent absence is a visible state rather than simulated activity.

## Session information

Record the reproduction command and arguments with sensitive values redacted, working directory, command outcome, timestamps, repository commit if present, and whether the checkout had uncommitted changes. A commit alone does not identify a dirty working tree: use a local snapshot/diff digest when supported, otherwise state that exact code identity is unavailable. Support non-Git projects.

Capture status is independent of command success: distinguish not started, collecting, settled after the configured grace period, failed, empty, and partial. A settled receiver does not prove all application spans were emitted. Track known dropped/rejected records and missing parent references; mark unknowns as unknown.

Record how telemetry was attributed to the reproduction: explicit session markers or a dedicated source are stronger than time-window overlap. Include exporter/service identity and retain ambiguity for concurrent activity. Do not promise global completeness from one local receiver.

Represent a source expectation as its description, origin (user task or test/code contract), relevant scope, evaluation method, and evidence. Runtime checks complement functional assertions; span success status does not prove the output is correct. Avoid requiring duration budgets unless specified or justified by the task.

## Analysis semantics

Group spans by trace ID across services and batches; identify spans by trace ID plus span ID. Preserve links, out-of-order arrival, and invalid references without inventing parents. Detect duplicate/conflicting records explicitly.

For parent interval coverage, clip child intervals to the parent and union them before subtracting their coverage. Label the remainder as time outside recorded child spans. Avoid summing overlapping durations as end-to-end latency or presenting a timestamp gap as proven network/queue delay. Missing dependency information can make an async critical path indeterminate.

Operation grouping should expose the normalized key and examples. Match repeated SQL/HTTP operations using available semantic attributes; return reduced confidence or unsupported analysis when attributes are absent. Distinguish observed repetition from inferred retries, redundancy, or N+1 behavior.

Differences should use comparable request scopes, retain unmatched operations, and disclose ambiguity. Separate deterministic counts from noisy latency observations. Keep analysis output tied to original evidence so both humans and agents can challenge it.

## Capture wrapper integration

The capture wrapper is a separate execution capability. It should use the project's existing command, show test output, return a compact session summary with evidence references, and make collector errors distinguishable from test failures. Endpoint configuration should apply only to the intended reproduction; do not overwrite shared/global environment settings or existing telemetry routing silently.

The desktop app starts and manages a loopback-bound receiver. Report receiver readiness, coding-agent connection/activity, test outcome, and telemetry state independently. If the app is unavailable or its port is occupied, return an actionable state; never silently send development telemetry to a remote fallback. Explicit container networking configuration may be needed for test services.

Instrumented applications may need exporter shutdown/flush support. Use a configurable bounded grace period and report late/partial evidence. Printing a session summary is the primary opportunity to bring evidence into the agent's normal workflow without requiring it to remember an extra tool.

## Skill and workflow enforcement

The skill guides model decisions; it is not an enforcement mechanism. A dedicated diagnosis runner may validate that evidence references exist and that verification was attempted or explicitly limited by missing data. It cannot prove a correct diagnosis from invocation records alone. Do not impose this gate on unrelated repository work.

Before release, validate skill examples against actual tool schemas/help and evaluate whether agents reach supported conclusions on incomplete, noisy, and misleading captures. Test the skill on the behavioral cases in the project README.
