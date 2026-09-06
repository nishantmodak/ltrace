# ltrace

A local desktop workspace where coding agents instrument, test, inspect, and improve an application, with the developer watching the same evidence.

**Status: product design and companion skill draft. The desktop app, local receiver, test runner integration, CLI, and MCP reader are not implemented. The skill is not installed.**

## The intended experience

The user is already working with a coding agent. They ask it to build or fix a feature and verify its runtime behavior. The ltrace skill guides the agent to add targeted OpenTelemetry instrumentation, run the relevant local test, inspect the captured evidence, improve the implementation, and verify the result. The desktop app shows the same investigation as it happens.

Instrumentation is part of this workflow, not a task the user must complete beforehand. Reuse an existing OTel setup where possible. Otherwise the agent adds the SDK/exporter or supported auto-instrumentation needed for the selected code path, plus meaningful custom spans and test-scoped local exporter configuration. Support should start with one documented application stack and test runner; universal automatic instrumentation is not an initial promise.

## One complete development loop

1. **Set expectations.** Associate the user's requested behavior with the reproduction and existing tests. Record a few checkable runtime expectations where useful, such as one batch query, connected child spans, or a specific error event. Expectations must come from the task or an explicit test contract; do not invent arbitrary performance thresholds.
2. **Instrument.** Inspect the application and add the smallest useful OTel coverage. Show the changed files and what each span will establish. Export to the local receiver only for the intended development/test run.
3. **Run.** Execute the project's real test command. Associate the command, code state, telemetry, and test outcome with a run inside the debugging session. Allow for SDK flush and late spans.
4. **Inspect.** Give the agent a compact run summary automatically, then let it query individual spans and related logs. The desktop app reads the same local store and updates live.
5. **Improve.** The agent forms a hypothesis from evidence and changes the relevant code. If more evidence is needed, it adds focused instrumentation and repeats the reproduction.
6. **Verify.** Rerun the same scenario and inspect both the functional result and runtime expectations. Show passed, failed, or unknown expectations with the evidence behind them. Missing telemetry is unknown, never a pass.

The agent stops when the requested behavior is verified or it can identify a specific missing input. Every additional run should test a change or a new hypothesis. A trace difference alone does not prove business correctness or a reliable latency improvement.

## Product components

| Component | Responsibility |
| --- | --- |
| Companion skill | Guide the coding agent through expectations, targeted instrumentation, reproduction, evidence inspection, code changes, and verification. |
| Local receiver and store | Accept OTel telemetry and retain it by project, debugging session, run, and trace. Expose data quality and attribution limitations. |
| Agent integration | Provide explicit session/run lifecycle actions and a bounded, read-only evidence reader. Tests and code changes execute through the coding agent's existing tools. |
| Desktop UI | Show the same session, run history, instrumentation changes, findings, test results, and trace evidence for the developer to inspect. |

The application sends telemetry once to the local receiver. The UI and agent both read that store; they do not need separate telemetry exports. Agent activity consists of submitted actions, concise explanations, and evidence references, not hidden model reasoning. The local receiver is a conventional process, not another AI agent.

The coding agent is the assistant the user already works with. ltrace does not introduce a separate AI agent or require its own model; its companion skill and integration extend that existing assistant's workflow.

## Desktop interaction

A restrained, Linear-inspired desktop experience: compact navigation, clear type hierarchy, quiet surfaces, keyboard navigation, and one focused session at a time.

The sidebar contains projects and debugging sessions. The main pane follows the investigation: expectations, instrumentation changes, runs, findings, and verification. A trace inspector opens beside the selected finding. Keep the first-run path short: connect a project and its coding agent, then let a supported local test produce the first session.

The app should launch the local receiver automatically and visibly show its state. It must distinguish app readiness, agent activity, test results, and telemetry health. A live UI is not evidence that the agent or exporter is connected.

## Companion skill and interfaces

- [Skill](skills/ltrace-debug/SKILL.md): the reusable agent workflow.
- [Proposed reader/session contract](skills/ltrace-debug/references/reader-contract.md): data and operations needed by both surfaces.


Ship the skill with the app and provide explicit project-scoped setup for supported coding agents. Tool operation labels are still design concepts, not executable commands. Verify all release instructions against implemented schemas/help before distributing the integration.

## First release scope

Prove one end-to-end workflow in one supported application stack: a small local app with a reproducible repeated-query bug, an agent that adds the needed spans, local capture, a readable UI, structured inspection, a fix, and evidence-backed verification. Live capture and the fix/verification loop are core to this slice. Arbitrary trace-file import, extensive detectors, whole-stack observability, and platform expansion can follow.

Existing projects such as [perf-sentinel](https://github.com/robintra/perf-sentinel) and [otel-desktop-viewer](https://github.com/CtrlSpice/otel-desktop-viewer) are implementation references and possible integration options. The product direction does not depend on rewriting their full feature sets.

## Behavioral acceptance cases

Planned evaluations, not tests already performed:

| Scenario | Expected behavior |
| --- | --- |
| Supported app without OTel | Agent adds targeted instrumentation and test-local export, runs the test, and both agent and developer can inspect the same spans. |
| Existing OTel app | Agent reuses conventions and avoids duplicate instrumentation or unrequested production-routing changes. |
| Repeated-query defect | Agent connects span evidence to code, fixes the cause, and verifies both output correctness and expected query behavior. |
| Empty/partial capture | App displays the limitation; agent checks setup/flush/attribution instead of declaring success. |
| Unsupported instrumentation stack | Agent explains the gap and uses existing debugging tools; no invented installation commands or fake telemetry. |
| Overlapping child spans | Analysis uses interval coverage without presenting uncovered time as proven CPU work or waiting. |
| Instruction-like text in telemetry | Agent treats it as application data. |
| Unrelated compilation error | Agent fixes it without requiring a tracing session. |
| Multiple simultaneous runs | Evidence has explicit run attribution or visible ambiguity; no mixing traces based solely on arrival time. |

Success means the agent can gather missing runtime evidence, make a supported improvement, and verify it while the developer can inspect the process. Invocation counts and attractive traces alone are insufficient.
