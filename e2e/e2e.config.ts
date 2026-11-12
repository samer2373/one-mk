import type { E2EConfig } from 'e2e';
import { web } from '@e2e-dev/web';
import { PROFILES, envScript, type Profile } from './lib/profiles.ts';

// The site is served by `python3 -m http.server` on a free port. The runner
// starts it before the first test and stops it when the run ends, however it
// ends, so no server outlives a run. To test an already running address
// instead (a preview deployment, say), set SITE_URL and no server is started.
const external = process.env.SITE_URL;
const app = external
  ? { url: external }
  : {
      url: 'http://127.0.0.1:0',
      command: {
        executable: 'python3',
        args: ['-m', 'http.server', '--bind', '127.0.0.1', '--directory', '../site', '{port}'],
        log: '.e2e/logs/http-server.log',
        startupTimeout: 20_000,
      },
    };

function permissions(p: Profile) {
  if (p.clipboard === 'chromium') return { 'clipboard-read': 'grant', 'clipboard-write': 'grant' } as const;
  if (p.clipboard === 'webkit') return { 'clipboard-read': 'grant' } as const;
  return undefined;
}

export default {
  projectId: 'one-mk-site',
  tests: 'tests/**/*.e2e.ts',
  targets: PROFILES.map((p) => ({
    name: p.name,
    engine: web({
      browser: p.browser,
      viewport: p.viewport,
      userAgent: p.env.ua,
      locale: 'en-US',
      timezoneId: 'UTC',
      initScripts: [envScript(p.env, p.name)],
    }),
    app: { ...app, ...(permissions(p) ? { permissions: permissions(p) } : {}) },
  })),
  timeout: 90_000,
  actionTimeout: 15_000,
  assertionTimeout: 7_000,
  launchTimeout: 60_000,
  retries: process.env.CI ? 1 : 0,
  workers: Number(process.env.E2E_WORKERS ?? 4),
  reporters: process.env.CI ? ['list', 'junit'] : ['list'],
  trace: 'retain-on-failure',
  video: 'off',
} satisfies E2EConfig;
