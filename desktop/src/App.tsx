import { useEffect, useState } from "react";
import { api } from "./api";
import { useLive } from "./useLive";
import {
  duration,
  timeOf,
  type SessionDetail,
  type Summary,
  type RecentTracePage,
  type RecentTrace,
} from "./domain";
import { ConnectionDialog, RunDialog, TracePanel } from "./panels";

export default function App() {
  const connection = useLive("connection", api.status);
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setSearch(query), 200);
    return () => clearTimeout(timer);
  }, [query]);
  const traces = useLive<RecentTracePage>(`traces:${search}`, () =>
    api.read(`traces?limit=200&q=${encodeURIComponent(search)}`),
  );
  const [selection, setSelection] = useState<RecentTrace | null>(null);
  const selected =
    traces.data?.traces.find(
      (t) =>
        t.run_id === selection?.run_id && t.trace_id === selection?.trace_id,
    ) ?? selection;
  useEffect(() => {
    if (!selection && traces.data?.traces[0])
      setSelection(traces.data.traces[0]);
  }, [selection, traces.data]);
  const runId = selected?.run_id ?? null;
  const report = useLive<Summary>(runId, () => api.read(`runs/${runId}`));
  const project = report.data?.run.session_id ?? null;
  const detail = useLive<SessionDetail>(project, () =>
    api.read(`sessions/${project}`),
  );
  const [requestedSpan, setRequestedSpan] = useState<string | null>(null);
  const [showConnection, setShowConnection] = useState(false);
  const [showRun, setShowRun] = useState(false);
  const [evidenceError, setEvidenceError] = useState("");
  const error =
    connection.error ||
    traces.error ||
    report.error ||
    detail.error ||
    evidenceError;
  const failures =
    report.data?.verification.filter((v) => v.status === "failed").length ?? 0;
  const unknown =
    report.data?.verification.filter((v) => v.status === "unknown").length ?? 0;
  async function inspectEvidence(reference: string) {
    const [trace, span] = reference.split("/");
    try {
      const page = await api.read<RecentTracePage>(
        `traces?limit=200&q=${trace}`,
      );
      const target = page.traces.find(
        (t) => t.run_id === runId && t.trace_id === trace,
      );
      if (!target) throw new Error("Referenced trace was not found.");
      setSelection(target);
      setRequestedSpan(span);
      setShowRun(false);
      setEvidenceError("");
    } catch (error) {
      setEvidenceError(String(error));
    }
  }
  return (
    <div className="app">
      <main>
        <header className="toolbar">
          <div className="brand">
            <svg viewBox="0 0 20 20" aria-hidden="true">
              <path d="M4 3v14h13M8 7h9M8 12h6" />
            </svg>
            <span>ltrace</span>
          </div>
          <div className="toolbar-actions">
            <span className="connection-status">
              <span
                className={`dot ${connection.data && !connection.error ? "ok" : "warning"}`}
              />
              {connection.data && !connection.error
                ? "Receiver connected"
                : "Receiver unavailable"}
            </span>
            <button
              aria-label="Connection details"
              className="icon-button"
              onClick={() => setShowConnection(true)}
            >
              ⚙
            </button>
          </div>
        </header>
        {error && (
          <div className="error-banner" role="alert">
            {error}
            <button
              onClick={() => {
                connection.refresh();
                detail.refresh();
                report.refresh();
                traces.refresh();
                referenced.refresh();
              }}
            >
              Retry
            </button>
          </div>
        )}
        {!selected && !traces.data?.total && !search ? (
          <div className="empty-state">
            <h1>Waiting for traces</h1>
            <p>
              Point your OpenTelemetry exporter at the local receiver.
              <br />
              Requests will appear here as they arrive.
            </p>
            <code>
              {connection.data?.url ?? "http://127.0.0.1:4318"}/v1/traces
            </code>
            <button
              className="text-button"
              onClick={() => setShowConnection(true)}
            >
              Connection details ↗
            </button>
          </div>
        ) : (
          <>
            {runId && (
              <div className="run-context">
                <div>
                  {report.data?.run.test_status === "not_run" ? (
                    <>
                      <span className="dot live" />
                      Live telemetry{" "}
                      <span className="muted">· Not linked to a test</span>
                    </>
                  ) : (
                    <>
                      <span
                        className={`dot ${report.data?.run.test_status === "passed" ? "ok" : "warning"}`}
                      />
                      <span>{report.data?.run.label ?? "Loading run…"}</span>
                      <span className="muted">
                        · Test {report.data?.run.test_status ?? "running"}
                      </span>
                      {failures > 0 && (
                        <button
                          className="failed-text"
                          onClick={() => setShowRun(true)}
                        >
                          · {failures} expectation{failures === 1 ? "" : "s"}{" "}
                          failed
                        </button>
                      )}
                      {!failures && unknown > 0 && (
                        <button
                          className="muted"
                          onClick={() => setShowRun(true)}
                        >
                          · {unknown} unverified
                        </button>
                      )}
                      {!failures &&
                        !unknown &&
                        !!report.data?.verification.length && (
                          <span className="ok-text">· Expectations passed</span>
                        )}
                    </>
                  )}
                  {!!report.data?.issues.length && (
                    <button
                      className="warning-text"
                      onClick={() => setShowRun(true)}
                    >
                      · Capture needs attention
                    </button>
                  )}
                </div>
                <button
                  className="text-button"
                  onClick={() => setShowRun(true)}
                >
                  Run details
                </button>
              </div>
            )}
            <div className="trace-workspace">
              <section className="trace-list" aria-label="Trace list">
                <div className="list-tools">
                  <input
                    aria-label="Search traces"
                    placeholder="Search traces…"
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                  />
                  <span>
                    {traces.data?.total ?? 0}{" "}
                    {(traces.data?.total ?? 0) === 1 ? "trace" : "traces"}
                  </span>
                </div>
                <div className="trace-table">
                  <div className="trace-items">
                    {traces.data?.traces.map((t) => (
                      <button
                        key={`${t.run_id}:${t.trace_id}`}
                        className={`trace-item ${selected?.run_id === t.run_id && selected?.trace_id === t.trace_id ? "selected" : ""}`}
                        aria-pressed={
                          selected?.run_id === t.run_id &&
                          selected?.trace_id === t.trace_id
                        }
                        onClick={() => {
                          setSelection(t);
                          setRequestedSpan(null);
                        }}
                      >
                        <span className="trace-item-title">
                          <span
                            className={`dot ${t.errors ? "error" : "neutral"}`}
                          />
                          <strong title={t.name}>
                            {t.name || "Unnamed span"}
                          </strong>
                          <span>{duration(t.duration_ns)}</span>
                        </span>
                        <span className="trace-item-service">
                          {t.services.join(", ")}
                        </span>
                        <span className="trace-item-meta">
                          <span>{timeOf(t.start_ns)}</span>
                          <span>
                            {t.span_count} spans
                            {t.errors ? ` · ${t.errors} errors` : ""}
                          </span>
                        </span>
                      </button>
                    ))}
                  </div>
                  {traces.data?.traces.length === 0 && (
                    <div className="list-empty">
                      {search
                        ? "No traces match this search."
                        : report.data?.run.capture_status === "empty"
                          ? "No spans received. Check instrumentation and SDK flush."
                          : "Waiting for trace evidence…"}
                    </div>
                  )}
                  {traces.data &&
                    traces.data.total > traces.data.traces.length && (
                      <p className="list-footnote">
                        Showing the latest 200 matching traces. Narrow the
                        search or use the paginated CLI reader.
                      </p>
                    )}
                </div>
              </section>
              {runId && selected ? (
                <TracePanel
                  key={`${runId}:${selected.trace_id}`}
                  runId={runId}
                  trace={selected}
                  requestedSpan={requestedSpan}
                />
              ) : (
                <div className="waterfall-empty">
                  Select a trace to inspect its waterfall.
                </div>
              )}
            </div>
          </>
        )}
      </main>
      {showConnection && (
        <ConnectionDialog
          connection={connection.data}
          onClose={() => setShowConnection(false)}
        />
      )}
      {showRun && report.data && detail.data && (
        <RunDialog
          summary={report.data}
          detail={detail.data}
          onClose={() => setShowRun(false)}
          onEvidence={inspectEvidence}
          onNote={() => detail.refresh()}
        />
      )}
    </div>
  );
}
