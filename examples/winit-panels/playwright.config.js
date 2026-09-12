import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests/browser",
  workers: 1,
  use: {
    channel: "chrome",
    baseURL: "http://127.0.0.1:8768",
    viewport: { width: 1280, height: 900 },
    deviceScaleFactor: 1,
    launchOptions: {
      args: ["--enable-unsafe-webgpu", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
    },
    screenshot: "only-on-failure",
  },
  webServer: {
    command: "python3 -m http.server 8768 --bind 127.0.0.1",
    url: "http://127.0.0.1:8768",
    reuseExistingServer: !process.env.CI,
  },
});
