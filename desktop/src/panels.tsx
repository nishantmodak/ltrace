import { useEffect, useRef, useState, type ReactNode } from "react";
import { api } from "./api";
import { useLive } from "./useLive";
import {
  attributeValue,
  captureCommand,
  compareOperations,
  duration,
  orderedSpans,
  spanDuration,
  timeline,
  type Connection,
  type FindingsReport,
  type Inspected,
  type SessionDetail,
  type Span,
  type SpanPage,
  type Summary,
  type Trace,
} from "./domain";

export function Dialog({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    ref.current?.showModal();
  }, []);
  return (
    <dialog ref={ref} onCancel={onClose}>
      <header>
        <h2>{title}</h2>
        <button
          className="icon-button"
          aria-label={`Close ${title}`}
          onClick={onClose}
        >
          ×
        </button>
      </header>
      <div className="dialog-body">{children}</div>
    </dialog>
  );
}
export function ConnectionDialog({
  connection,
  onClose,
}: {
  connection: Connection | null;
  onClose: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState("");
  async function copy() {
    try {
      await navigator.clipboard.writeText(captureCommand(connection));
      setCopied(true);
    } catch {
      setError("Select the command below to copy it.");
    }
  }
  return (
    <Dialog title="Local receiver" onClose={onClose}>
      <p>
        Traces appear automatically when your application exports to this
        endpoint.
      </p>
      <label className="field-label">OTLP HTTP endpoint</label>
      <code className="copyable">
        {connection?.url ?? "http://127.0.0.1:4318"}/v1/traces
      </code>
      <details className="disclosure">
        <summary>Exporter configuration</summary>
        <pre>{`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=${connection?.url ?? "http://127.0.0.1:4318"}/v1/traces\nOTEL_EXPORTER_OTLP_TRACES_PROTOCOL=http/protobuf`}</pre>
        <p>
          Use your existing SDK. Flush it before a short-lived process exits.
        </p>
      </details>
      <div className="dialog-section">
        <h3>Capture a test</h3>
        <p>
          Your coding agent can run a test with isolated telemetry and read its
          results. Project and run information is recorded automatically.
        </p>
        <code className="copyable">{captureCommand(connection)}</code>
        <button className="small-button" onClick={() => void copy()}>
          {copied ? "Copied" : "Copy command"}
        </button>
        {error && <p role="alert">{error}</p>}
      </div>
      {connection?.skill_path && (
        <details className="disclosure">
          <summary>Companion skill</summary>
          <p>
            Give this file to your coding agent for instrumentation, inspection,
            and verification guidance.
          </p>
          <code className="copyable">{connection.skill_path}</code>
        </details>
      )}
    </Dialog>
  );
}

export function TracePanel({
  runId,
  trace,
  requestedSpan,
  evidenceNonce,
}: {
  runId: string;
  trace: Trace;
  requestedSpan: string | null;
  evidenceNonce: number;
}) {
  const [spanId, setSpanId] = useState<string | null>(requestedSpan);
  const [findingId, setFindingId] = useState<string | null>(null);
  const findings = useLive<FindingsReport>(
    `${runId}:${trace.trace_id}:${trace.span_count}:findings`,
    () => api.read(`runs/${runId}/traces/${trace.trace_id}/findings`),
    0,
  );
  const activeFinding = findings.data?.findings.find((f) => f.id === findingId);
  const highlighted = new Set(activeFinding?.span_ids ?? []);
  useEffect(() => {
    if (spanId)
      document
        .querySelector(`[data-span-id="${spanId}"]`)
        ?.scrollIntoView?.({ block: "nearest" });
  }, [findingId, spanId]);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [extra, setExtra] = useState<Span[]>([]);
  const [next, setNext] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    setSpanId(requestedSpan);
  }, [runId, trace.trace_id, trace.root_span_id, requestedSpan, evidenceNonce]);
  useEffect(() => {
    setExtra([]);
    setNext(null);
    setError("");
  }, [runId, trace.trace_id, trace.root_span_id]);
  const page = useLive<SpanPage>(
    `${runId}:${trace.trace_id}:${trace.span_count}`,
    () => api.read(`runs/${runId}/spans?trace_id=${trace.trace_id}&limit=200`),
    0,
  );
  useEffect(() => {
    setExtra([]);
    setNext(page.data?.next_offset ?? null);
  }, [page.data]);
  const inspected = useLive<Inspected>(
    spanId ? `${runId}:${trace.trace_id}:${spanId}` : null,
    () => api.read(`runs/${runId}/spans/${trace.trace_id}/${spanId}`),
    0,
  );
  const spans = [...(page.data?.spans ?? []), ...extra];
  const selection = useRef(`${runId}:${trace.trace_id}:${trace.span_count}`);
  selection.current = `${runId}:${trace.trace_id}:${trace.span_count}`;
  async function more() {
    if (next === null) return;
    const key = selection.current;
    setBusy(true);
    try {
      const result = await api.read<SpanPage>(
        `runs/${runId}/spans?trace_id=${trace.trace_id}&limit=200&offset=${next}`,
      );
      if (selection.current === key) {
        setExtra((items) => [...items, ...result.spans]);
        setNext(result.next_offset);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  const ordered = orderedSpans(spans);
  const parents = new Map(spans.map((s) => [s.span_id, s.parent_span_id]));
  const branches = new Set(spans.map((s) => s.parent_span_id));
  const visible = ordered.filter(({ span }) => {
    const seen = new Set<string>([span.span_id]);
    let parent = span.parent_span_id;
    while (parent && !seen.has(parent)) {
      if (collapsed.has(parent)) return false;
      seen.add(parent);
      parent = parents.get(parent) ?? "";
    }
    return true;
  });
  // Use the complete trace bounds even when only a page of spans is loaded.
  const bounds = {
    trace_id: trace.trace_id,
    start_ns: trace.start_ns,
    end_ns: (
      BigInt(trace.start_ns) + BigInt(trace.duration_ns ?? "0")
    ).toString(),
  } as Span;
  return (
    <div className="trace-view">
      <section className="trace-panel" aria-label="Trace inspector">
        <header>
          <div>
            <h2>{trace.name}</h2>
            <span>
              {trace.services.join(", ")} <b>·</b> {duration(trace.duration_ns)}{" "}
              <b>·</b> {trace.span_count} spans
            </span>
          </div>
        </header>
        <div className="trace-reference">
          Trace <code>{trace.trace_id}</code>
        </div>
        {(page.error || error) && (
          <div className="error-banner" role="alert">
            {page.error || error}
          </div>
        )}
        <section className="findings-strip" aria-label="Trace findings">
          <div className="findings-label">
            Findings{" "}
            {findings.data && <span>{findings.data.findings.length}</span>}
          </div>
          {findings.error ? (
            <p role="alert">
              {findings.error}{" "}
              <button className="text-button" onClick={findings.refresh}>
                Retry findings
              </button>
            </p>
          ) : findings.data ? (
            <>
              <div className="finding-chips">
                {findings.data.findings.map((finding) => (
                  <button
                    key={finding.id}
                    className={`finding-chip ${finding.id === findingId ? "active" : ""}`}
                    aria-pressed={finding.id === findingId}
                    onClick={() => {
                      setFindingId(finding.id);
                      setSpanId(finding.span_ids[0] ?? null);
                      setCollapsed(new Set());
                    }}
                  >
                    {finding.title}
                  </button>
                ))}
              </div>
              {!findings.data.findings.length && (
                <p>No findings in recorded spans.</p>
              )}
              {!!findings.data.limitations.length && (
                <details className="finding-limits">
                  <summary>Detection limits</summary>
                  {findings.data.limitations.map((limit) => (
                    <p key={limit}>{limit}</p>
                  ))}
                </details>
              )}
            </>
          ) : (
            <p>Checking recorded spans…</p>
          )}
        </section>
        <div className="waterfall" aria-label="Span waterfall">
          <div className="waterfall-head">
            <span>Operation</span>
            <span className="timeline-axis">
              <span>0</span>
              <span>{duration(trace.duration_ns)}</span>
            </span>
            <span>Duration</span>
          </div>
          {visible.map(({ span, depth }) => {
            const bar = timeline(span, [bounds]);
            const branch = branches.has(span.span_id);
            return (
              <div
                key={span.span_id}
                data-span-id={span.span_id}
                className={`span-row ${spanId === span.span_id ? "selected" : ""} ${highlighted.has(span.span_id) ? "finding-match" : ""}`}
              >
                <div
                  className="span-name"
                  style={{ paddingLeft: 8 + depth * 12 }}
                >
                  {branch ? (
                    <button
                      className="branch-toggle"
                      aria-label={`${collapsed.has(span.span_id) ? "Expand" : "Collapse"} ${span.name}`}
                      aria-expanded={!collapsed.has(span.span_id)}
                      onClick={() =>
                        setCollapsed((old) => {
                          const next = new Set(old);
                          if (next.has(span.span_id)) next.delete(span.span_id);
                          else next.add(span.span_id);
                          return next;
                        })
                      }
                    >
                      <svg
                        viewBox="0 0 14 14"
                        aria-hidden="true"
                        focusable="false"
                      >
                        <path d="m5 3 4 4-4 4" />
                      </svg>
                    </button>
                  ) : (
                    <span className="branch-spacer" />
                  )}
                  <button
                    className="span-select"
                    title={`${span.name} · ${span.service}`}
                    onClick={() => setSpanId(span.span_id)}
                  >
                    <span
                      className={`dot ${span.error ? "error" : "neutral"}`}
                    />
                    <span>{span.name}</span>
                  </button>
                </div>
                <button
                  className="bar-track"
                  aria-label={`Inspect ${span.name}, ${spanDuration(span)}`}
                  onClick={() => setSpanId(span.span_id)}
                >
                  <i
                    className={span.error ? "error-bar" : ""}
                    style={{ left: `${bar.left}%`, width: `${bar.width}%` }}
                  />
                </button>
                <span className="span-duration">{spanDuration(span)}</span>
              </div>
            );
          })}
          {next !== null && (
            <button
              className="load-more"
              disabled={busy}
              onClick={() => void more()}
            >
              Load more spans ({spans.length} of {trace.span_count})
            </button>
          )}
        </div>
      </section>
      {spanId && (
        <aside className="span-inspector" aria-label="Selected span">
          <header>
            <span>Span details</span>
            <button
              className="icon-button"
              aria-label="Close span details"
              onClick={() => {
                setSpanId(null);
                setFindingId(null);
              }}
            >
              ×
            </button>
          </header>
          {activeFinding && (
            <section
              className="finding-explanation"
              aria-label="Finding evidence"
            >
              <h3>{activeFinding.title}</h3>
              <p>{activeFinding.explanation}</p>
              <p>{activeFinding.suggestion}</p>
              <div className="finding-evidence-links">
                {activeFinding.span_ids.slice(0, 20).map((id, i) => (
                  <button
                    key={id}
                    aria-label={`Inspect evidence span ${i + 1}`}
                    aria-pressed={spanId === id}
                    onClick={() => setSpanId(id)}
                  >
                    Span {i + 1}
                  </button>
                ))}
              </div>
              {activeFinding.span_count > 20 && (
                <p>
                  Showing the first 20 evidence links.{" "}
                  {activeFinding.span_count} spans matched.
                </p>
              )}
            </section>
          )}
          {inspected.error ? (
            <div className="error-banner" role="alert">
              {inspected.error}
            </div>
          ) : inspected.data ? (
            <SpanDetails inspected={inspected.data} />
          ) : (
            <div className="list-empty">Loading span…</div>
          )}
        </aside>
      )}
    </div>
  );
}

function SpanDetails({ inspected }: { inspected: Inspected }) {
  const span = inspected.span;
  const attributes = Array.isArray(span.raw.attributes)
    ? (span.raw.attributes as { key: string; value: unknown }[])
    : [];
  return (
    <section className="span-details" aria-label="Span details">
      <div className="span-heading">
        <h3>{span.name}</h3>
        <span>
          {span.error ? (
            <span className="failed-text">Error</span>
          ) : (
            spanDuration(span)
          )}
        </span>
      </div>
      <div className="span-detail-scroll">
        <h4>
          Attributes <span>{attributes.length}</span>
        </h4>
        {attributes.length ? (
          <dl className="attributes">
            {attributes.map((a, i) => (
              <div key={`${a.key}:${i}`}>
                <dt>{a.key}</dt>
                <dd>{attributeValue(a.value)}</dd>
              </div>
            ))}
          </dl>
        ) : (
          <p className="muted">No attributes recorded.</p>
        )}
        <details className="disclosure">
          <summary>Timing and identifiers</summary>
          <dl className="attributes">
            <div>
              <dt>Span ID</dt>
              <dd>{span.span_id}</dd>
            </div>
            <div>
              <dt>Parent span</dt>
              <dd>{span.parent_span_id || "Root span"}</dd>
            </div>
            <div>
              <dt>Start · unix ns</dt>
              <dd>{span.start_ns}</dd>
            </div>
            <div>
              <dt>Outside child spans</dt>
              <dd>{duration(inspected.uncovered_recorded_ns)}</dd>
            </div>
          </dl>
          <p>{inspected.coverage_note}</p>
        </details>
        {(
          [
            ["Events", span.raw.events],
            ["Links", span.raw.links],
            ["Resource", span.resource],
            ["Instrumentation scope", span.scope],
            ["Raw span", span.raw],
          ] as const
        ).map(([name, value]) => (
          <details className="disclosure" key={name}>
            <summary>
              {name}
              {Array.isArray(value) ? ` · ${value.length}` : ""}
            </summary>
            <pre>{JSON.stringify(value, null, 2)}</pre>
          </details>
        ))}
      </div>
    </section>
  );
}

export function RunDialog({
  summary,
  detail,
  onClose,
  onEvidence,
  onNote,
}: {
  summary: Summary;
  detail: SessionDetail;
  onClose: () => void;
  onEvidence: (reference: string) => void;
  onNote: () => void;
}) {
  const [baselineId, setBaselineId] = useState("");
  const baseline = useLive<Summary>(
    baselineId || null,
    () => api.read(`runs/${baselineId}`),
    0,
  );
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function submit() {
    setBusy(true);
    try {
      await api.note(detail.session.id, note, summary.run.id);
      setNote("");
      onNote();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog title="Run details" onClose={onClose}>
      <div className="run-title">
        <h3>{summary.run.label}</h3>
        <span className="muted">
          {summary.span_count} spans · {summary.trace_count} traces
        </span>
      </div>
      <dl className="run-facts">
        <div>
          <dt>Project</dt>
          <dd>{detail.session.project}</dd>
        </div>
        <div>
          <dt>Test</dt>
          <dd>
            {summary.run.test_status === "not_run"
              ? "No test attribution"
              : `${summary.run.test_status} · ${summary.run.exit_code === null ? "no exit code" : `exit ${summary.run.exit_code}`}`}
          </dd>
        </div>
        <div>
          <dt>Capture</dt>
          <dd>{summary.run.capture_status}</dd>
        </div>
        <div>
          <dt>Command</dt>
          <dd>{summary.run.command}</dd>
        </div>
        <div>
          <dt>Revision</dt>
          <dd>{summary.run.revision}</dd>
        </div>
        <div>
          <dt>Run ID</dt>
          <dd>{summary.run.id}</dd>
        </div>
      </dl>
      {summary.issues.length > 0 && (
        <div className="quality-note">
          <strong>Evidence needs attention</strong>
          {summary.issues.map((i) => (
            <p key={i}>{i}</p>
          ))}
        </div>
      )}
      <div className="dialog-section">
        <h3>Runtime expectations</h3>
        {summary.verification.length === 0 ? (
          <p>No expectations were attached to this run.</p>
        ) : (
          summary.verification.map((v, i) => (
            <details className="expectation" key={i}>
              <summary>
                <span
                  className={`dot ${v.status === "passed" ? "ok" : v.status === "failed" ? "error" : "warning"}`}
                />
                <strong>{v.name}</strong>
                <span>{v.status}</span>
              </summary>
              <div>
                <p>{v.reason}</p>
                <p>
                  {v.observed_count} observed · {v.expected_min}–
                  {v.expected_max} expected
                </p>
                <p>{v.explanation}</p>
                <div className="evidence-links">
                  {v.evidence.map((id) => (
                    <button key={id} onClick={() => onEvidence(id)}>
                      {id.split("/")[1].slice(0, 8)} ↗
                    </button>
                  ))}
                </div>
              </div>
            </details>
          ))
        )}
      </div>
      {detail.runs.length > 1 && (
        <div className="dialog-section">
          <label className="section-with-control">
            <h3>Compare operations</h3>
            <select
              aria-label="Compare to"
              value={baselineId}
              onChange={(e) => setBaselineId(e.target.value)}
            >
              <option value="">Choose a baseline</option>
              {detail.runs
                .filter((r) => r.id !== summary.run.id)
                .map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.label}
                  </option>
                ))}
            </select>
          </label>
          {baseline.error && <p role="alert">{baseline.error}</p>}
          {baseline.data && (
            <>
              <p>
                Use equivalent workloads, instrumentation, and expectations.
                Counts describe recorded evidence.
              </p>
              {summary.issues.length ||
              baseline.data.issues.length ||
              summary.run.capture_status !== "settled" ||
              baseline.data.run.capture_status !== "settled" ||
              summary.operations_total > 100 ||
              baseline.data.operations_total > 100 ? (
                <p className="warning-text">
                  Capture quality or truncated operation groups limits this
                  comparison.
                </p>
              ) : null}
              <table className="comparison-table">
                <thead>
                  <tr>
                    <th>Operation</th>
                    <th>Before</th>
                    <th>After</th>
                  </tr>
                </thead>
                <tbody>
                  {compareOperations(summary, baseline.data).map((o) => (
                    <tr key={JSON.stringify([o.service, o.name])}>
                      <td>
                        {o.name}
                        <small>{o.service}</small>
                      </td>
                      <td>{o.before ?? "Unknown"}</td>
                      <td>{o.after ?? "Unknown"}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}
        </div>
      )}
      <div className="dialog-section">
        <h3>Notes</h3>
        {detail.notes
          .filter((n) => !n.run_id || n.run_id === summary.run.id)
          .map((n) => (
            <article className="note" key={n.id}>
              {n.body}
            </article>
          ))}
        <textarea
          aria-label="Add a note"
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="An observation, a change, or the next thing to check."
          maxLength={8000}
        />
        <div className="note-actions">
          <span>{error && <span role="alert">{error}</span>}</span>
          <button
            className="small-button"
            disabled={busy || !note.trim()}
            onClick={() => void submit()}
          >
            Add note
          </button>
        </div>
      </div>
    </Dialog>
  );
}
