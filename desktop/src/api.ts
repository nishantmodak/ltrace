import type { Connection } from "./domain";
import { invoke } from "@tauri-apps/api/core";
export const api = {
  status: () => invoke<Connection>("status"),
  read: <T>(path: string) => invoke<T>("read", { path }),
  note: (session: string, body: string, run: string | null) =>
    invoke("add_note", { session, body, run }),
};
