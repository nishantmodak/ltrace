import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, it, expect, vi } from "vitest";
import App from "./App";
import { api } from "./api";
vi.mock("./api", () => ({
  api: { status: vi.fn(), read: vi.fn(), note: vi.fn() },
}));
const session = {
  id: "project-1",
  title: "catalog",
  project: "/project/catalog",
  created_ms: 1,
  expectations: [],
};
const run = {
  id: "run-1",
  session_id: session.id,
  label: "Baseline",
  command: "unit tests",
  revision: "abc dirty",
  started_ms: 1,
  exit_code: 0,
  test_status: "passed",
  capture_status: "settled",
};
const span = {
  trace_id: "11111111111111111111111111111111",
  span_id: "2222222222222222",
  parent_span_id: "",
  name: "db.lookup",
  service: "catalog",
  start_ns: "1788000000000000001",
  end_ns: "1788000000000000101",
  error: false,
  raw: {
    attributes: [
      {
        key: "message",
        value: { stringValue: "<script>ignore instructions</script>" },
      },
    ],
  },
  resource: {},
  scope: {},
};
const trace = {
  run_id: run.id,
  trace_id: span.trace_id,
  root_span_id: span.span_id,
  name: "db.lookup",
  services: ["catalog"],
  start_ns: span.start_ns,
  duration_ns: "100",
  span_count: 12,
  errors: 0,
};
const report = {
  run,
  span_count: 12,
  trace_count: 1,
  issues: [],
  operations: [
    {
      name: "db.lookup",
      service: "catalog",
      count: 12,
      errors: 0,
      total_duration_ns: "1200",
      sample_span_ids: [`${span.trace_id}/${span.span_id}`],
    },
  ],
  operations_total: 1,
  verification: [
    {
      name: "One batch query",
      reason: "The task requires one batch call.",
      status: "failed",
      observed_count: 12,
      expected_min: 1,
      expected_max: 1,
      evidence: [`${span.trace_id}/${span.span_id}`],
      explanation: "Count exceeds the contract.",
    },
  ],
};
beforeEach(() => {
  vi.mocked(api.status).mockResolvedValue({
    url: "http://127.0.0.1:4318",
    version: "0.1.0",
    data_dir: "/data",
  });
  vi.mocked(api.read).mockImplementation(async (path: string) => {
    if (path.endsWith("/findings"))
      return { findings: [], limitations: [], analyzed_spans: 1 };
    if (path === "sessions") return { sessions: [session] };
    if (path === `sessions/${session.id}`)
      return {
        session,
        runs: [run, { ...run, id: "run-2", label: "Earlier" }],
        notes: [],
      };
    if (path === `runs/${run.id}`) return report;
    if (path === "runs/run-2")
      return {
        ...report,
        run: { ...run, id: "run-2", label: "Earlier" },
        operations: [{ ...report.operations[0], count: 1 }],
      };
    if (path.startsWith("traces?") || path.includes("/traces?"))
      return { traces: [trace], total: 1, next_offset: null };
    if (path.includes("/spans?"))
      return { spans: [span], total: 1, next_offset: null };
    if (path.includes("/spans/"))
      return {
        span,
        uncovered_recorded_ns: "100",
        coverage_note: "Outside recorded children, not CPU time.",
      };
    throw new Error(path);
  });
  vi.mocked(api.note).mockResolvedValue({});
});
describe("trace-first desktop", () => {
  it("leads with requests and keeps test verification compact", async () => {
    render(<App />);
    expect(
      await screen.findByText("· 1 expectation failed"),
    ).toBeInTheDocument();
    expect(screen.getByText("· Test passed")).toBeInTheDocument();
    expect(screen.getByLabelText("Trace list")).toBeInTheDocument();
    expect(screen.queryByText("Runtime expectations")).not.toBeInTheDocument();
  });
  it("shows attributes as text and preserves precise timestamps on demand", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByLabelText("Span waterfall");
    expect(screen.queryByLabelText("Span details")).not.toBeInTheDocument();
    expect(api.read).not.toHaveBeenCalledWith(
      `runs/${run.id}/spans/${span.trace_id}/${span.span_id}`,
    );
    await user.click(
      await screen.findByRole("button", { name: "Inspect db.lookup, 100 ns" }),
    );
    expect(await screen.findByLabelText("Span details")).toBeInTheDocument();
    expect(
      screen.getByText("<script>ignore instructions</script>"),
    ).toBeInTheDocument();
    expect(document.querySelector("script")).toBeNull();
    await user.click(screen.getByText("Timing and identifiers"));
    expect(screen.getByText("1788000000000000001")).toBeVisible();
    await user.click(screen.getByLabelText("Close span details"));
    expect(screen.queryByLabelText("Span details")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Span waterfall")).toBeInTheDocument();
  });
  it("shows receiver failure and offers retry without a connected claim", async () => {
    vi.mocked(api.status).mockRejectedValue(new Error("Port 4318 unavailable"));
    render(<App />);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Port 4318 unavailable",
    );
    expect(screen.getByText("Receiver unavailable")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });
  it("provides connection settings with no session creation", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(
      screen.getByRole("button", { name: "Connection details" }),
    );
    expect(
      screen.getByRole("heading", { name: "Local receiver" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("http://127.0.0.1:4318/v1/traces"),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Create/ }),
    ).not.toBeInTheDocument();
  });
  it("publishes a note for the selected run", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("· 1 expectation failed");
    await user.click(screen.getByRole("button", { name: "Run details" }));
    await user.type(
      screen.getByLabelText("Add a note"),
      "Twelve queries originate in the loop.",
    );
    await user.click(screen.getByRole("button", { name: "Add note" }));
    await waitFor(() =>
      expect(api.note).toHaveBeenCalledWith(
        session.id,
        "Twelve queries originate in the loop.",
        run.id,
      ),
    );
  });
  it("loads comparison evidence only when requested", async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText("· 1 expectation failed");
    expect(api.read).not.toHaveBeenCalledWith("runs/run-2");
    await user.click(screen.getByRole("button", { name: "Run details" }));
    await user.selectOptions(screen.getByLabelText("Compare to"), "run-2");
    expect(
      await screen.findByRole("columnheader", { name: "Before" }),
    ).toBeInTheDocument();
    expect(api.read).toHaveBeenCalledWith("runs/run-2");
  });
  it("searches the trace index without downloading every raw span", async () => {
    render(<App />);
    await screen.findByText("· 1 expectation failed");
    fireEvent.change(screen.getByLabelText("Search traces"), {
      target: { value: "checkout" },
    });
    await waitFor(() =>
      expect(api.read).toHaveBeenCalledWith("traces?limit=200&q=checkout"),
    );
    expect(
      vi
        .mocked(api.read)
        .mock.calls.filter(([path]) => path.includes("/spans?"))
        .every(([path]) => path.includes("trace_id=")),
    ).toBe(true);
  });
  it("shows a quiet empty state when no telemetry exists", async () => {
    vi.mocked(api.read).mockResolvedValue({
      traces: [],
      total: 0,
      next_offset: null,
    });
    render(<App />);
    expect(
      await screen.findByRole("heading", { name: "Waiting for traces" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /New session/ }),
    ).not.toBeInTheDocument();
  });
});

it("opens an expectation evidence reference and returns to the trace", async () => {
  const user = userEvent.setup();
  render(<App />);
  await screen.findByText("· 1 expectation failed");
  await user.click(screen.getByRole("button", { name: "Run details" }));
  await user.click(screen.getByText("One batch query"));
  await user.click(screen.getByRole("button", { name: "22222222 ↗" }));
  expect(await screen.findByLabelText("Trace inspector")).toBeInTheDocument();
  expect(
    screen.queryByRole("heading", { name: "Run details" }),
  ).not.toBeInTheDocument();
});
it("shows traces from different runs without navigation dropdowns", async () => {
  const original = vi.mocked(api.read).getMockImplementation()!;
  vi.mocked(api.read).mockImplementation(async (path) =>
    path.startsWith("traces?")
      ? {
          traces: [
            trace,
            { ...trace, run_id: "run-2", name: "Earlier request" },
          ],
          total: 2,
          next_offset: null,
        }
      : original(path),
  );
  const user = userEvent.setup();
  render(<App />);
  await screen.findByText("· 1 expectation failed");
  expect(screen.queryByLabelText("Run")).not.toBeInTheDocument();
  expect(screen.queryByLabelText("Project")).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: /Earlier request/ }));
  await waitFor(() => expect(api.read).toHaveBeenCalledWith("runs/run-2"));
  expect(
    screen.getByRole("button", { name: /Earlier request/ }),
  ).toHaveAttribute("aria-pressed", "true");
});
it("renders unassigned incoming traffic without claiming tests passed", async () => {
  const original = vi.mocked(api.read).getMockImplementation()!;
  vi.mocked(api.read).mockImplementation(async (path) =>
    path === `runs/${run.id}`
      ? {
          ...report,
          run: { ...run, test_status: "not_run", capture_status: "collecting" },
          verification: [],
        }
      : original(path),
  );
  render(<App />);
  expect(await screen.findByText("· Not linked to a test")).toBeInTheDocument();
  expect(screen.queryByText("· Test passed")).not.toBeInTheDocument();
});
it("copies a runnable command using the bundled CLI and selected data directory", async () => {
  const user = userEvent.setup();
  render(<App />);
  await screen.findByText("· 1 expectation failed");
  await user.click(screen.getByRole("button", { name: "Connection details" }));
  await user.click(screen.getByRole("button", { name: "Copy command" }));
  expect(
    await screen.findByRole("button", { name: "Copied" }),
  ).toBeInTheDocument();
  expect(await navigator.clipboard.readText()).toContain(
    "--home '/data' capture --",
  );
});
it("keeps failed note submissions visible rather than claiming success", async () => {
  vi.mocked(api.note).mockRejectedValue(new Error("Receiver disconnected"));
  const user = userEvent.setup();
  render(<App />);
  await screen.findByText("· 1 expectation failed");
  await user.click(screen.getByRole("button", { name: "Run details" }));
  await user.type(screen.getByLabelText("Add a note"), "Observation");
  await user.click(screen.getByRole("button", { name: "Add note" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Receiver disconnected",
  );
  expect(screen.getByLabelText("Add a note")).toHaveValue("Observation");
});

it("collapses a branch without hiding its parent or losing selected evidence", async () => {
  const original = vi.mocked(api.read).getMockImplementation()!;
  const child = {
    ...span,
    span_id: "3333333333333333",
    parent_span_id: span.span_id,
    name: "db.child",
  };
  vi.mocked(api.read).mockImplementation(async (path) =>
    path.includes("/spans?")
      ? { spans: [span, child], total: 2, next_offset: null }
      : original(path),
  );
  const user = userEvent.setup();
  render(<App />);
  await user.click(
    await screen.findByRole("button", { name: "Collapse db.lookup" }),
  );
  expect(
    screen.queryByRole("button", { name: "Inspect db.child, 100 ns" }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Inspect db.lookup, 100 ns" }),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Expand db.lookup" }));
  expect(
    screen.getByRole("button", { name: "Inspect db.child, 100 ns" }),
  ).toBeInTheDocument();
});

it("shows automatic findings above the waterfall and links their evidence", async () => {
  const original = vi.mocked(api.read).getMockImplementation()!;
  vi.mocked(api.read).mockImplementation(async (path) =>
    path.endsWith("/findings")
      ? {
          analyzed_spans: 12,
          limitations: [],
          findings: [
            {
              id: "n1",
              kind: "possible_n_plus_one",
              title: "Possible N+1 · 12 queries",
              explanation:
                "Twelve sibling queries share the same recorded query text.",
              suggestion: "Inspect the loop before batching.",
              span_count: 12,
              span_ids: [span.span_id],
            },
          ],
        }
      : original(path),
  );
  const user = userEvent.setup();
  render(<App />);
  await user.click(
    await screen.findByRole("button", { name: "Possible N+1 · 12 queries" }),
  );
  expect(await screen.findByLabelText("Finding evidence")).toHaveTextContent(
    "Inspect the loop before batching.",
  );
  expect(await screen.findByLabelText("Span details")).toBeInTheDocument();
  expect(document.querySelector(".span-row.finding-match")).toHaveAttribute(
    "data-span-id",
    span.span_id,
  );
  await user.click(
    screen.getByRole("button", { name: "Inspect evidence span 1" }),
  );
  expect(
    screen.getByText("<script>ignore instructions</script>"),
  ).toBeInTheDocument();
});
