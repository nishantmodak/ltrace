import { render, screen, act, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { TracePanel } from "./panels";
import { api } from "./api";
import type { Span, Trace, SpanPage } from "./domain";

vi.mock("./api", () => ({
  api: { status: vi.fn(), read: vi.fn(), note: vi.fn() },
}));

const runId = "run-1";
const traceIdA = "11111111111111111111111111111111";

function span(i: number, traceId = traceIdA): Span {
  const id = `s${String(i).padStart(6, "0")}`;
  return {
    trace_id: traceId,
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

function makeTrace(traceId = traceIdA, count = 250): Trace {
  return {
    trace_id: traceId,
    root_span_id: "s000000",
    name: "trace",
    services: ["catalog"],
    start_ns: String(10n ** 20n),
    duration_ns: "100",
    span_count: count,
    errors: 0,
  };
}

// The inspected endpoint (the /spans/{trace}/{span} path) must respond so the
// SpanDetails pane hydrates; its payload is irrelevant to pagination.
const inspected = {
  span: span(0),
  uncovered_recorded_ns: null,
  coverage_note: "",
};

function firstPage(traceId: string): SpanPage {
  return {
    spans: Array.from({ length: 200 }, (_, i) => span(i, traceId)),
    total: 250,
    next_offset: 200,
  };
}

// The stale "Load more" response: a tail page computed against the OLD total
// (250), so its cursor is null, claiming the trace has been fully read.
function staleTailPage(traceId: string): SpanPage {
  return {
    spans: Array.from({ length: 50 }, (_, i) => span(200 + i, traceId)),
    total: 250,
    next_offset: null,
  };
}

// Mount (or rerender) and flush one microtask inside `act` so the page hook's
// async fetch resolves and React commits the [page.data] effect before we read
// the DOM synchronously. Driving the initial render through `findByRole`'s
// `waitFor` interleaves `act` boundaries across timer ticks in a way that, under
// v8 coverage instrumentation, defeats useLive's keyed commit and makes the
// panel remount in a loop. An explicit single `act` flush is deterministic.
async function mount(node: React.ReactElement) {
  let view!: ReturnType<typeof render>;
  await act(async () => {
    view = render(node);
    await Promise.resolve();
  });
  return view;
}

describe("TracePanel 'Load more' staleness", () => {
  it(
    "drops an in-flight 'Load more' response when the live trace grows (span_count changes)",
    async () => {
    let resolveMore!: (value: SpanPage) => void;
    const morePromise = new Promise<SpanPage>((resolve) => {
      resolveMore = resolve;
    });
    vi.mocked(api.read).mockImplementation(async (path: string) => {
      if (path.includes("/spans/")) return inspected;
      if (path.includes("offset=")) return morePromise; // stale more() — held pending
      if (path.includes("/spans?")) return firstPage(traceIdA); // fresh first page (offset 0)
      throw new Error(path);
    });

    const view = await mount(
      <TracePanel
        runId={runId}
        trace={makeTrace(traceIdA, 250)}
        requestedSpan={null}
      />,
    );
    const loadMore = screen.getByRole("button", { name: /Load more spans/ });
    expect(loadMore).toHaveTextContent("Load more spans (200 of 250)");

    // Click "Load more": more() captures the selection key (run-1:traceA:250)
    // and issues the offset fetch, which we hold pending.
    await act(async () => {
      fireEvent.click(loadMore);
    });

    // A 2s trace-list poll delivers a grown trace; span_count 250 -> 300. The
    // page key changes -> useLive refetches offset 0 and the [page.data] effect
    // resets extra=[] and next=200 against the new span_count.
    await act(async () => {
      view.rerender(
        <TracePanel
          runId={runId}
          trace={makeTrace(traceIdA, 300)}
          requestedSpan={null}
        />,
      );
      await Promise.resolve();
    });
    const refreshed = screen.getByRole("button", { name: /Load more spans/ });
    expect(refreshed).toHaveTextContent("Load more spans (200 of 300)");

    // The stale offset response (computed against the old 250-span total) now
    // resolves. Its guard must reject it because the selection identity changed.
    await act(async () => {
      resolveMore(staleTailPage(traceIdA));
      await Promise.resolve();
    });

    // FIX: the stale response is dropped — the fresh cursor (200) is preserved,
    // the "Load more" button remains visible, and the stale tail spans are not
    // appended. (Before the fix the button vanished and the badge read 250/300.)
    const kept = screen.getByRole("button", { name: /Load more spans/ });
    expect(kept).toHaveTextContent("Load more spans (200 of 300)");
    expect(kept).not.toBeDisabled();
    expect(screen.queryByText("span-200")).not.toBeInTheDocument();
    expect(screen.queryByText("span-249")).not.toBeInTheDocument();
  },
  15000,
  );

  it("applies a fresh 'Load more' response when span_count is unchanged", async () => {
    vi.mocked(api.read).mockImplementation(async (path: string) => {
      if (path.includes("/spans/")) return inspected;
      if (path.includes("offset=")) return staleTailPage(traceIdA); // immediate resolve
      if (path.includes("/spans?")) return firstPage(traceIdA);
      throw new Error(path);
    });

    await mount(
      <TracePanel
        runId={runId}
        trace={makeTrace(traceIdA, 250)}
        requestedSpan={null}
      />,
    );
    const loadMore = screen.getByRole("button", { name: /Load more spans/ });

    // Click "Load more": selection identity is unchanged, so the response must
    // be applied (spans appended, cursor advances to null at the end).
    await act(async () => {
      fireEvent.click(loadMore);
      await Promise.resolve();
    });

    expect(
      screen.queryByRole("button", { name: /Load more spans/ }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("span-200")).toBeInTheDocument();
    expect(screen.getByText("span-249")).toBeInTheDocument();
  });
});
