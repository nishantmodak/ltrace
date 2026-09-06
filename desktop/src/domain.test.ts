import { describe, it, expect } from "vitest";
import {
  duration,
  spanDuration,
  orderedSpans,
  timeline,
  compareOperations,
  type Span,
  type Summary,
} from "./domain";
const span = (
  id: string,
  parent = "",
  start = "1788000000000000001",
  end = "1788000000000000101",
): Span => ({
  trace_id: "trace",
  span_id: id,
  parent_span_id: parent,
  start_ns: start,
  end_ns: end,
  name: id,
  service: "catalog",
  error: false,
  dropped: false,
  raw: {},
  resource: {},
  scope: {},
});
describe("trace arithmetic", () => {
  it("keeps precision beyond JavaScript safe integers", () => {
    expect(spanDuration(span("a"))).toBe("100 ns");
    expect(duration("1000000")).toBe("1.00 ms");
    expect(duration("999999")).toBe("1000.0 µs");
    expect(duration("1000000000")).toBe("1.00 s");
    expect(duration(null)).toBe("—");
    expect(spanDuration(span("a", "", "2", "1"))).toBe("—");
  });
  it("lays out children within each independent trace", () => {
    const root = span("root");
    const child = span(
      "child",
      "root",
      "1788000000000000026",
      "1788000000000000051",
    );
    expect(timeline(child, [root, child])).toEqual({ left: 25, width: 25 });
  });
  it("orders parents first and retains orphans and cycles exactly once", () => {
    const rows = orderedSpans([
      span("child", "root"),
      span("a", "b"),
      span("root"),
      span("b", "a"),
      span("orphan", "missing"),
    ]);
    expect(rows).toHaveLength(5);
    expect(rows.find((x) => x.span.span_id === "child")?.depth).toBe(1);
    expect(new Set(rows.map((x) => x.span.span_id)).size).toBe(5);
  });
  it("handles deep malformed traces without recursion or unlimited indentation", () => {
    const rows = orderedSpans(
      Array.from({ length: 3000 }, (_, i) => span(`${i}`, i ? `${i - 1}` : "")),
    );
    expect(rows).toHaveLength(3000);
    expect(Math.max(...rows.map((x) => x.depth))).toBe(12);
  });
  it("does not merge equal operation names from different services", () => {
    const summary = (service: string, count: number) =>
      ({
        operations: [{ service, name: "lookup", count }],
        run: {},
      }) as Summary;
    const result = compareOperations(summary("a", 1), summary("b", 12));
    expect(result).toHaveLength(2);
    expect(result[0]).toMatchObject({ service: "b", before: 12, after: 0 });
  });
});

it("does not invent zero counts for groups omitted from a truncated summary", () => {
  const current = {
    operations: [{ service: "a", name: "lookup", count: 1 }],
    operations_total: 1,
  } as Summary;
  const baseline = {
    operations: [],
    operations_total: 101,
  } as unknown as Summary;
  expect(compareOperations(current, baseline)[0].before).toBeNull();
});
