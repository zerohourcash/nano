import { defineConfig, devices } from '@playwright/test'
import os from 'node:os'
import path from 'node:path'

const port = 18177
const dbPath = path.join(os.tmpdir(), `meshkeeper-playwright-${process.pid}.db`)
process.env.MESHKEEPER_E2E_DB = dbPath

export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [['list'], ['html', { open: 'never', outputFolder: 'test-results/html' }]],
  outputDir: 'test-results/artifacts',
  globalTeardown: './e2e/global-teardown.ts',
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: 'node scripts/start.mjs',
    url: `http://127.0.0.1:${port}/health`,
    timeout: 30_000,
    reuseExistingServer: false,
    env: {
      MESHKEEPER_DB: dbPath,
      MESHKEEPER_BIND: `127.0.0.1:${port}`,
      MESHKEEPER_DEMO_DATA: '0',
      MESHKEEPER_DEMO_LOGIN: '0',
      MESHKEEPER_OPEN_REGISTRATION: '0',
      MESHKEEPER_SYNC_TOKEN: 'playwright-offline-bundle-token-at-least-32-chars',
    },
  },
})
