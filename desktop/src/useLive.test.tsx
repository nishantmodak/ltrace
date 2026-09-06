import { render, screen, act } from "@testing-library/react";
import { it, expect } from "vitest";
import { useLive } from "./useLive";
it("ignores a late response from a previously selected record", async () => {
  let resolveOld!: (value: string) => void;
  const old = new Promise<string>((resolve) => {
    resolveOld = resolve;
  });
  function View({ id }: { id: string }) {
    const result = useLive(
      id,
      () => (id === "old" ? old : Promise.resolve("current record")),
      0,
    );
    return <div>{result.data}</div>;
  }
  const { rerender } = render(<View id="old" />);
  rerender(<View id="new" />);
  expect(await screen.findByText("current record")).toBeInTheDocument();
  await act(async () => resolveOld("stale record"));
  expect(screen.queryByText("stale record")).not.toBeInTheDocument();
});
