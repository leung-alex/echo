import { defineConfig } from "@playwright/test";

const url = "http://127.0.0.1:5187";

export default defineConfig({
  testDir: ".",
  testMatch: "**/*.spec.ts",
  timeout: 30_000,
  expect: { timeout: 5_000 },
  fullyParallel: false,
  workers: 1,
  reporter: "line",
  use: {
    baseURL: url,
    trace: "retain-on-failure",
  },
  webServer: {
    command:
      "pnpm --dir ../../frontend/app exec vite --host 127.0.0.1 --port 5187",
    url,
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
