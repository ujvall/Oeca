// oeca: the tests import through Nuxt's `~` / `@` aliases (app/); outside Nuxt
// they need saying. No Nuxt runtime: the tested code is plain TypeScript.
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

const app = fileURLToPath(new URL("./app", import.meta.url));

export default defineConfig({
  resolve: { alias: { "~": app, "@": app } },
  test: { include: ["app/**/*.test.ts", "*.test.ts"] },
});
