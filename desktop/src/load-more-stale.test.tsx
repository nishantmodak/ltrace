import {
  render,
  screen,
  act,
  waitFor,
  fireEvent,
} from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import App from "./App";
import { api } from "./api";
import type { Span, RecentTrace, SpanPage } from "./domain";

vi.mock("./api", () => ({
  api: { status: vi.fn(), read: vi.fn(), note: vi.fn() },
}));

const runId = "run-1";
const traceId = "11111111111111111111111111111111";

const session = {
  id: "project-1",
  title: "catalog",
  project: "/project/catalog",
  created_ms: 1,
  expectations: [],
};

// The run is actively capturing ("collecting"), so the live span set is still
// growing: store.rs:188 initializes capture_status "collecting" at run start
// and analysis.rs:111 treats "settled" as the completeness criterion.
const run = {
  id: runId,
  session_id: session.id,
  label: "Baseline",
  command: "unit tests",
  revision: "abc dirty",
  started_ms: 1,
  finished_ms: null,
  exit_code: 0,
  test_status: "passed",
  capture_status: "collecting",
  issues: [],
};

const report = {
  run,
  span_count: 250,
  trace_count: 1,
  issues: [],
  operations: [],
  operations_total: 0,
  verification: [],
};

function span(i: number, tid = traceId): Span {
  const id = `s${String(i).padStart(6, "0")}`;
  return {
    trace_id: tid,
    span_id: id,
    parent_span_id: i === 0 ? "" : `s${String(i - 1).padStart(6, "0")}`,
    name: `span-${i}`,
    service: "catalog",
    start_ns: String(10n ** 20n + BigInt(i) * 1000n),
    end_ns: String(10n ** 20n + BigInt(i) * 1000n + 100n),
    error: false,
    dropped: false,
    raw: {},
    resource: {},
    scope: {},
  };
}

const trace: RecentTrace = {
  run_id: runId,
  trace_id: traceId,
  root_span_id: "s000000",
  name: "trace",
  services: ["catalog"],
  start_ns: String(10n ** 20n),
  duration_ns: "100",
  span_count: 250,
  errors: 0,
};

const inspected = {
  span: span(0),
  uncovered_recorded_ns: null,
  coverage_note: "",
};

// First page is fetched when the trace has 250 spans: total reflects the live
// COUNT(*) at fetch time, and next_offset stays open at 200.
function firstPage(tid: string): SpanPage {
  return {
    spans: Array.from({ length: 200 }, (_, i) => span(i, tid)),
    total: 250,
    next_offset: 200,
  };
}

// By the time the user clicks "Load more" the trace has grown to 600 spans.
// The offset=200 page reports the authoritative live COUNT (`total: 600`) and
// keeps paging open at 400.
function grownPage(tid: string): SpanPage {
  return {
    spans: Array.from({ length: 200 }, (_, i) => span(200 + i, tid)),
    total: 600,
    next_offset: 400,
  };
}

beforeEach(() => {
  vi.mocked(api.status).mockResolvedValue({
    url: "http://127.0.0.1:4318",
    version: "0.1.0",
    data_dir: "/data",
  });
  vi.mocked(api.read).mockImplementation(async (path: string) => {
    // A non-matching search excludes the selected trace from the polled
    // recent-traces list, mirroring store.rs:397's HAVING filter.
    if (path === "traces?limit=200&q=xyz")
      return { traces: [], total: 0, next_offset: null };
    if (path.startsWith("traces?") || path.includes("/traces?"))
      return { traces: [trace], total: 1, next_offset: null };
    if (path === `sessions/${session.id}`)
      return { session, runs: [run], notes: [] };
    if (path === `runs/${runId}`) return report;
    if (path.endsWith("/findings"))
      return { findings: [], limitations: [], analyzed_spans: 0 };
    if (path.includes("/spans/")) return inspected;
    if (path.includes("offset=")) return grownPage(traceId);
    if (path.includes("/spans?")) return firstPage(traceId);
    throw new Error(`unmocked path: ${path}`);
  });
  vi.mocked(api.note).mockResolvedValue({});
});

describe("TracePanel 'Load more' denominator while a trace is filtered out", () => {
  it("uses the live spans-endpoint total rather than the frozen span_count", async () => {
    render(<App />);

    // The single trace (span_count 250) is auto-selected; the first spans
    // page returns total 250 with next_offset 200, so the badge is visible.
    const loadMore = await screen.findByRole(
      "button",
      { name: /Load more spans/ },
      { timeout: 5000 },
    );
    expect(loadMore).toHaveTextContent("Load more spans (200 of 250)");

    // Type a non-matching search so the selected trace disappears from the
    // polled recent-traces list; App falls back to the frozen `selection` and
    // trace.span_count stops updating.
    fireEvent.change(screen.getByLabelText("Search traces"), {
      target: { value: "xyz" },
    });
    await waitFor(() =>
      expect(api.read).toHaveBeenCalledWith("traces?limit=200&q=xyz"),
    );
    // Confirm the filter is committed (the trace list now shows the
    // no-match message) and the selected trace is frozen out of the list.
    await waitFor(() =>
      expect(
        screen.getByText("No traces match this search."),
      ).toBeInTheDocument(),
    );

    // Click "Load more": the live spans endpoint now sees a grown trace
    // (total 600) and returns next_offset 400, so the button stays visible.
    await act(async () => {
      fireEvent.click(loadMore);
      await Promise.resolve();
    });

    const after = screen.getByRole("button", { name: /Load more spans/ });
    // The badge must not show the impossible "loaded exceeds total" count
    // against the stale denominator; it must reflect the live total (600).
    expect(after).not.toHaveTextContent("400 of 250");
    expect(after).toHaveTextContent("Load more spans (400 of 600)");
    // The header `N spans` must agree with the badge's denominator (600
    // spans), not the frozen span_count (250 spans).
    expect(screen.getByText(/600 spans/)).toBeInTheDocument();
    expect(screen.queryByText(/250 spans/)).not.toBeInTheDocument();
  }, 15000);
});
