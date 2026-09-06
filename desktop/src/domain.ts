export type Expectation = {
  name: string;
  reason: string;
  service: string;
  operation: string;
  min_count: number;
  max_count: number;
};
export type Session = {
  id: string;
  title: string;
  project: string;
  created_ms: number;
  expectations: Expectation[];
};
export type Run = {
  id: string;
  session_id: string;
  label: string;
  command: string;
  revision: string;
  started_ms: number;
  finished_ms: number | null;
  exit_code: number | null;
  test_status: string;
  capture_status: string;
  issues: string[];
};
export type Note = {
  id: string;
  created_ms: number;
  body: string;
  run_id: string | null;
};
export type SessionDetail = { session: Session; runs: Run[]; notes: Note[] };
export type Operation = {
  service: string;
  name: string;
  count: number;
  errors: number;
  total_duration_ns: string;
  sample_span_ids: string[];
};
export type Verification = {
  name: string;
  reason: string;
  status: string;
  observed_count: number;
  expected_min: number;
  expected_max: number;
  evidence: string[];
  explanation: string;
};
export type Summary = {
  run: Run;
  span_count: number;
  trace_count: number;
  issues: string[];
  operations: Operation[];
  operations_total: number;
  verification: Verification[];
};
export type Span = {
  trace_id: string;
  span_id: string;
  parent_span_id: string;
  name: string;
  service: string;
  start_ns: string;
  end_ns: string;
  error: boolean;
  dropped: boolean;
  raw: Record<string, unknown>;
  resource: unknown;
  scope: unknown;
};
export type SpanPage = {
  spans: Span[];
  total: number;
  next_offset: number | null;
};
export type Inspected = {
  span: Span;
  uncovered_recorded_ns: string | null;
  coverage_note: string;
};
export function duration(ns: string | null | undefined): string {
  if (ns == null || !/^\d+$/.test(ns)) return "—";
  const value = BigInt(ns);
  if (value < 1000n) return `${value} ns`;
  if (value < 1000000n) return `${(Number(value) / 1000).toFixed(1)} µs`;
  if (value < 1000000000n) return `${(Number(value) / 1000000).toFixed(2)} ms`;
  return `${(Number(value) / 1000000000).toFixed(2)} s`;
}
export function spanDuration(span: Span): string {
  try {
    const n = BigInt(span.end_ns) - BigInt(span.start_ns);
    return n < 0n ? "—" : duration(n.toString());
  } catch {
    return "—";
  }
}
export function spanKey(span: Span): string {
  return `${span.trace_id}/${span.span_id}`;
}
export function orderedSpans(spans: Span[]): { span: Span; depth: number }[] {
  const ids = new Set(spans.map(spanKey));
  const children = new Map<string, Span[]>();
  const roots: Span[] = [];
  for (const span of spans) {
    const parent = `${span.trace_id}/${span.parent_span_id}`;
    if (span.parent_span_id && ids.has(parent)) {
      const list = children.get(parent) ?? [];
      list.push(span);
      children.set(parent, list);
    } else roots.push(span);
  }
  const compare = (a: Span, b: Span) =>
    a.start_ns.length - b.start_ns.length ||
    a.start_ns.localeCompare(b.start_ns) ||
    spanKey(a).localeCompare(spanKey(b));
  roots.sort(compare);
  for (const list of children.values()) list.sort(compare);
  const result: { span: Span; depth: number }[] = [];
  const seen = new Set<string>();
  // Iteration handles malformed cycles and deeply nested traces without recursion.
  const append = (root: Span) => {
    const stack = [{ span: root, depth: 0 }];
    while (stack.length) {
      const item = stack.pop()!;
      const key = spanKey(item.span);
      if (seen.has(key)) continue;
      seen.add(key);
      result.push(item);
      const list = children.get(key) ?? [];
      for (let i = list.length - 1; i >= 0; i--)
        stack.push({ span: list[i], depth: Math.min(item.depth + 1, 12) });
    }
  };
  for (const span of roots) append(span);
  for (const span of [...spans].sort(compare)) append(span);
  return result;
}
export function compareOperations(current: Summary, baseline: Summary) {
  const groups = new Map<
    string,
    {
      service: string;
      name: string;
      before: number | null;
      after: number | null;
    }
  >();
  for (const [summary, side] of [
    [baseline, "before"],
    [current, "after"],
  ] as const)
    for (const op of summary.operations) {
      const key = JSON.stringify([op.service, op.name]);
      const item = groups.get(key) ?? {
        service: op.service,
        name: op.name,
        before:
          baseline.operations_total > baseline.operations.length ? null : 0,
        after: current.operations_total > current.operations.length ? null : 0,
      };
      item[side] = op.count;
      groups.set(key, item);
    }
  return [...groups.values()].sort(
    (a, b) =>
      Math.abs((b.after ?? 0) - (b.before ?? 0)) -
        Math.abs((a.after ?? 0) - (a.before ?? 0)) ||
      a.name.localeCompare(b.name),
  );
}
export function timeline(
  span: Span,
  spans: Span[],
): { left: number; width: number } {
  try {
    const same = spans
      .filter((s) => s.trace_id === span.trace_id)
      .map((s) => [BigInt(s.start_ns), BigInt(s.end_ns)])
      .filter(([s, e]) => e >= s);
    const start = same.reduce(
      (m, [s]) => (s < m ? s : m),
      BigInt(span.start_ns),
    );
    const end = same.reduce((m, [, e]) => (e > m ? e : m), BigInt(span.end_ns));
    const total = end - start;
    if (total <= 0n) return { left: 0, width: 1 };
    const left = Math.max(
      0,
      Math.min(
        100,
        Number(((BigInt(span.start_ns) - start) * 10000n) / total) / 100,
      ),
    );
    const width = Math.max(
      0.5,
      Math.min(
        100 - left,
        Number(
          ((BigInt(span.end_ns) - BigInt(span.start_ns)) * 10000n) / total,
        ) / 100,
      ),
    );
    return { left, width };
  } catch {
    return { left: 0, width: 1 };
  }
}

export type Connection = {
  url: string;
  version: string;
  data_dir: string;
  cli_path?: string;
  skill_path?: string;
};
export function captureCommand(connection: Connection | null) {
  const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
  const executable = connection?.cli_path
    ? quote(connection.cli_path)
    : "ltrace-dev";
  const home = connection?.data_dir
    ? ` --home ${quote(connection.data_dir)}`
    : "";
  return `${executable}${home} capture -- <your-test-command>`;
}

export type Trace = {
  trace_id: string;
  root_span_id: string;
  name: string;
  services: string[];
  start_ns: string;
  duration_ns: string | null;
  span_count: number;
  errors: number;
};
export type TracePage = {
  traces: Trace[];
  total: number;
  next_offset: number | null;
};
export function timeOf(ns: string) {
  try {
    return new Date(Number(BigInt(ns) / 1000000n)).toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  } catch {
    return "—";
  }
}
export function attributeValue(value: unknown): string {
  if (value && typeof value === "object") {
    const v = value as Record<string, unknown>;
    for (const key of [
      "stringValue",
      "intValue",
      "doubleValue",
      "boolValue",
      "bytesValue",
    ])
      if (key in v) return String(v[key]);
  }
  return JSON.stringify(value) ?? "—";
}

export type RecentTrace = Trace & { run_id: string };
export type RecentTracePage = {
  traces: RecentTrace[];
  total: number;
  next_offset: number | null;
};

export type Finding = {
  id: string;
  kind: string;
  title: string;
  explanation: string;
  suggestion: string;
  span_count: number;
  span_ids: string[];
};
export type FindingsReport = {
  findings: Finding[];
  limitations: string[];
  analyzed_spans: number;
};
