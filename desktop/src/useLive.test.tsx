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
it("keeps the previous data while re-polling on a same-key refresh", async () => {
  const resolvers: Array<(value: string) => void> = [];
  let refresh!: () => void;
  function View() {
    const result = useLive(
      "k",
      () => new Promise<string>((resolve) => resolvers.push(resolve)),
      0,
    );
    refresh = result.refresh;
    return <div data-testid="val">{result.data}</div>;
  }
  render(<View />);
  await act(async () => resolvers.shift()!("data-1"));
  expect(screen.getByTestId("val")).toHaveTextContent("data-1");
  await act(async () => {
    refresh();
  });
  expect(screen.getByTestId("val")).toHaveTextContent("data-1");
  await act(async () => resolvers.shift()!("data-2"));
  expect(screen.getByTestId("val")).toHaveTextContent("data-2");
});
