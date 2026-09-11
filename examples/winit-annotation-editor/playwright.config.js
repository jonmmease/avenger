import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests/browser",
  workers: 1,
  use: {
    channel: "chrome",
    baseURL: "http://127.0.0.1:8767",
    viewport: { width: 1200, height: 840 },
    deviceScaleFactor: 1,
    launchOptions: {
      args: ["--enable-unsafe-webgpu", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
    },
    screenshot: "only-on-failure",
  },
  webServer: {
    command: "python3 -m http.server 8767 --bind 127.0.0.1",
    url: "http://127.0.0.1:8767",
    reuseExistingServer: !process.env.CI,
  },
});
