import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './tests/browser',
  timeout: 60_000,
  workers: 1,
  use: {
    channel: 'chrome',
    baseURL: 'http://127.0.0.1:8772',
    viewport: { width: 1120, height: 900 },
    launchOptions: { args: ['--enable-unsafe-webgpu', '--use-angle=swiftshader', '--enable-unsafe-swiftshader'] },
    screenshot: 'only-on-failure',
  },
  webServer: {
    command: 'python3 -m http.server 8772 --bind 127.0.0.1',
    url: 'http://127.0.0.1:8772',
    reuseExistingServer: false,
  },
});
