import { useEffect, useState } from "react";
import { api } from "./api";
import { useLive } from "./useLive";
import {
  duration,
  timeOf,
  type Session,
  type SessionDetail,
  type Summary,
  type TracePage,
  type Trace,
} from "./domain";
import { ConnectionDialog, RunDialog, TracePanel } from "./panels";

export default function App() {
  const connection = useLive("connection", api.status);
  const projects = useLive("projects", () =>
    api.read<{ sessions: Session[] }>("sessions"),
  );
  const [projectId, setProjectId] = useState<string | null>(null);
  useEffect(() => {
    if (!projectId && projects.data?.sessions[0])
      setProjectId(projects.data.sessions[0].id);
  }, [projectId, projects.data]);
  const project = projectId ?? projects.data?.sessions[0]?.id ?? null;
  const detail = useLive<SessionDetail>(project, () =>
    api.read(`sessions/${project}`),
  );
  const [selectedRun, setSelectedRun] = useState("latest");
  const runId =
    (selectedRun === "latest" ? detail.data?.runs[0]?.id : selectedRun) ?? null;
  const report = useLive<Summary>(runId, () => api.read(`runs/${runId}`));
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setSearch(query), 200);
    return () => clearTimeout(timer);
  }, [query]);
  const traces = useLive<TracePage>(runId ? `${runId}:${search}` : null, () =>
    api.read(`runs/${runId}/traces?limit=200&q=${encodeURIComponent(search)}`),
  );
  const [selectedTrace, setSelectedTrace] = useState<string | null>(null);
  const [requestedSpan, setRequestedSpan] = useState<string | null>(null);
  const [showConnection, setShowConnection] = useState(false);
  const [showRun, setShowRun] = useState(false);
  const [showInspector, setShowInspector] = useState(true);
  useEffect(() => {
    setSelectedTrace(null);
    setRequestedSpan(null);
  }, [runId]);
  const referenced = useLive<TracePage>(
    selectedTrace &&
      !traces.data?.traces.some((t) => t.trace_id === selectedTrace)
      ? `${runId}:${selectedTrace}`
      : null,
    () => api.read(`runs/${runId}/traces?q=${selectedTrace}`),
    0,
  );
  const selected: Trace | undefined =
    traces.data?.traces.find((t) => t.trace_id === selectedTrace) ??
    referenced.data?.traces[0] ??
    (!selectedTrace ? traces.data?.traces[0] : undefined);
  const error =
    connection.error ||
    projects.error ||
    detail.error ||
    report.error ||
    traces.error ||
    referenced.error;
  const failures =
    report.data?.verification.filter((v) => v.status === "failed").length ?? 0;
  const unknown =
    report.data?.verification.filter((v) => v.status === "unknown").length ?? 0;
  function switchProject(id: string) {
    setProjectId(id);
    setSelectedRun("latest");
    setQuery("");
    setSelectedTrace(null);
  }
  function inspectEvidence(reference: string) {
    const [trace, span] = reference.split("/");
    setQuery("");
    setSelectedTrace(trace);
    setRequestedSpan(span);
    setShowRun(false);
    setShowInspector(true);
  }
  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <path d="M4 3v14h13M8 7h9M8 12h6" />
          </svg>
          <span>ltrace</span>
        </div>
        <div className="nav-label">Projects</div>
        <nav aria-label="Projects">
          {projects.data?.sessions.map((p) => (
            <button
              key={p.id}
              onClick={() => switchProject(p.id)}
              className={project === p.id ? "active" : ""}
            >
              <span className="project-icon">
                {p.project === "ltrace:incoming" ? "↙" : "▧"}
              </span>
              <span title={p.project}>{p.title}</span>
            </button>
          ))}
        </nav>
        {!projects.data?.sessions.length && (
          <p className="sidebar-help">Projects appear as traces arrive.</p>
        )}
        <button
          className="receiver-button"
          onClick={() => setShowConnection(true)}
        >
          <span
            className={`dot ${connection.data && !connection.error ? "ok" : "warning"}`}
          />
          <span>
            {connection.data && !connection.error
              ? "Receiver connected"
              : "Receiver unavailable"}
            <small>
              {connection.data?.url.replace("http://", "") ??
                "Connection details"}
            </small>
          </span>
          <span>⌘</span>
        </button>
      </aside>
      <main>
        <header className="toolbar">
          <div className="breadcrumb">
            <strong>Traces</strong>
            {detail.data && (
              <>
                <span>/</span>
                <span>{detail.data.session.title}</span>
              </>
            )}
          </div>
          <div className="toolbar-actions">
            {detail.data?.runs.length ? (
              <select
                aria-label="Run"
                value={selectedRun}
                onChange={(e) => setSelectedRun(e.target.value)}
              >
                <option value="latest">Latest run</option>
                {detail.data.runs.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.label}
                  </option>
                ))}
              </select>
            ) : null}
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
                projects.refresh();
                detail.refresh();
                report.refresh();
                traces.refresh();
              }}
            >
              Retry
            </button>
          </div>
        )}
        {!runId ? (
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
              <button className="text-button" onClick={() => setShowRun(true)}>
                Run details
              </button>
            </div>
            <div className="trace-workspace">
              <section className="trace-list" aria-label="Trace list">
                <div className="list-tools">
                  <input
                    aria-label="Search traces"
                    placeholder="Search by operation, service, or trace ID"
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                  />
                  <span>
                    {traces.data?.total ?? 0}{" "}
                    {(traces.data?.total ?? 0) === 1 ? "trace" : "traces"}
                  </span>
                </div>
                <div className="trace-table">
                  <table>
                    <thead>
                      <tr>
                        <th>Request</th>
                        <th>Duration</th>
                        <th>Spans</th>
                        <th>Time</th>
                      </tr>
                    </thead>
                    <tbody>
                      {traces.data?.traces.map((t) => (
                        <tr
                          key={t.trace_id}
                          className={
                            selected?.trace_id === t.trace_id && showInspector
                              ? "selected"
                              : ""
                          }
                        >
                          <td>
                            <button
                              className="trace-name"
                              onClick={() => {
                                setSelectedTrace(t.trace_id);
                                setRequestedSpan(null);
                                setShowInspector(true);
                              }}
                            >
                              <span
                                className={`dot ${t.errors ? "error" : "neutral"}`}
                              />
                              <span>
                                <strong>{t.name || "Unnamed span"}</strong>
                                <small>{t.services.join(", ")}</small>
                              </span>
                            </button>
                          </td>
                          <td>{duration(t.duration_ns)}</td>
                          <td>{t.span_count}</td>
                          <td>{timeOf(t.start_ns)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
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
              {showInspector && selected && (
                <TracePanel
                  runId={runId}
                  trace={selected}
                  requestedSpan={requestedSpan}
                  onClose={() => setShowInspector(false)}
                />
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
