import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
export default defineConfig({
  plugins: [react()],
  server: { port: 1420, strictPort: true },
  clearScreen: false,
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
    restoreMocks: true,
    coverage: {
      provider: "v8",
      include: [
        "src/domain.ts",
        "src/App.tsx",
        "src/panels.tsx",
        "src/useLive.ts",
      ],
      reporter: ["text", "html"],
    },
  },
});
