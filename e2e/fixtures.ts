import { test as base, expect, type Page } from "@playwright/test";
import type { Scenario } from "../src/testing/mockBackend";

// Every test fails on an uncaught exception or a console error the test
// didn't expect: a crashed section or a swallowed failure is a bug even
// when the assertion under test passes.
export const test = base.extend<{ consoleErrors: string[] }>({
  consoleErrors: async ({ page }, use) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(`pageerror: ${err.message}`));
    page.on("console", (msg) => {
      if (msg.type() === "error") errors.push(msg.text());
    });
    await use(errors);
    expect(errors, "unexpected console errors").toEqual([]);
  },
});

export { expect };

export async function open(page: Page, window: "widget" | "settings", scenario: Scenario = {}) {
  await page.addInitScript((s) => {
    window.__DW_SCENARIO__ = s;
  }, scenario);
  await page.goto(`/?window=${window}`);
}

// Waits (briefly) for the page to subscribe to `event` first, so an event
// played right after `open` isn't lost on a slow machine. Some tests play
// events nothing listens to, so it plays it anyway after the wait.
export async function emit(page: Page, event: string, payload?: unknown) {
  await page
    .waitForFunction((e) => window.__dw?.listening.has(e) ?? false, event, { timeout: 2000 })
    .catch(() => {});
  return page.evaluate(([e, p]) => window.__dw!.emit(e as string, p), [event, payload] as const);
}

export function calls(page: Page, cmd: string) {
  return page.evaluate((c) => window.__dw!.calls.filter((x) => x.cmd === c).map((x) => x.args ?? null), cmd);
}
