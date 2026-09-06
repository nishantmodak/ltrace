import { useEffect, useRef, useState } from "react";
// A keyed request owns its response. Late responses from previous selections
// must never overwrite the currently inspected project or trace.
export function useLive<T>(
  key: string | null,
  load: () => Promise<T>,
  interval = 2000,
) {
  const reader = useRef(load);
  reader.current = load;
  const [state, setState] = useState<{
    key: string | null;
    data: T | null;
    error: string;
  }>({ key: null, data: null, error: "" });
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    if (!key) {
      setState({ key, data: null, error: "" });
      return;
    }
    setState({ key, data: null, error: "" });
    async function poll() {
      try {
        const data = await reader.current();
        if (active) setState({ key, data, error: "" });
      } catch (error) {
        if (active)
          setState((previous) => ({
            key,
            data: previous.key === key ? previous.data : null,
            error: String(error),
          }));
      }
      if (active && interval) timer = setTimeout(poll, interval);
    }
    void poll();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [key, interval, revision]);
  return {
    data: state.key === key ? state.data : null,
    error: state.key === key ? state.error : "",
    refresh: () => setRevision((n) => n + 1),
  };
}
