import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

const root = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  resolve: {
    alias: {
      "@bindings": path.join(root, "../apps/desktop/src/bindings"),
      "@shared": path.join(root, "../apps/desktop/src/shared"),
    },
  },
  test: { include: ["shared/src/**/*.test.ts"] },
});
