import type { Page } from "@playwright/test";
import { calls, emit, expect, open, test } from "./fixtures";

const agent = (over: Record<string, unknown> = {}) => ({
  session_id: "s1",
  cwd: "/Users/me/dev-whisper",
  folder: "dev-whisper",
  message: "Claude needs your permission to use Bash",
  kind: "needs_input",
  app_pid: 4242,
  bundle_id: "com.apple.Terminal",
  app_name: "Terminal",
  tty: "ttys004",
  received_ms: Date.now() - 2 * 60_000,
  ...over,
});

test.describe("coding agents: widget", () => {
  test.use({ viewport: { width: 280, height: 360 } });

  test("shows nothing when no agent is waiting", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await expect(page.getByText("Ready")).toBeVisible();
    await expect(page.getByTestId("agent-pill")).toHaveCount(0);
  });

  test("a waiting agent shows a pill that goes to its window @csp", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await emit(page, "agents-changed", [agent()]);

    const pill = page.getByTestId("agent-pill");
    await expect(pill).toHaveText("Claude is waiting · dev-whisper");
    await expect(pill).toHaveAttribute("title", /needs your permission.*Terminal.*⌃⌥R/);

    // Entering the widget opens its hover menu, which moves the row up; a
    // real pointer is already inside by the time it reaches the pill.
    await pill.hover();
    await pill.click();
    await expect.poll(() => calls(page, "focus_waiting_agent")).toEqual([{ sessionId: "s1" }]);
  });

  test("several agents show the newest and a count", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget", { agents: [agent({ session_id: "b", folder: "api" }), agent()] });
    await expect(page.getByTestId("agent-pill")).toHaveText("Claude is waiting · api +1");
  });

  test("the pill goes away once the agent is answered", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget", { agents: [agent()] });
    await expect(page.getByTestId("agent-pill")).toBeVisible();
    await emit(page, "agents-changed", []);
    await expect(page.getByTestId("agent-pill")).toHaveCount(0);
    await expect(page.getByText("Ready")).toBeVisible();
  });

  test("a reply that went to the agent says so", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget", { agents: [agent()] });
    await emit(page, "transcript-copied", "Sent to dev-whisper in Terminal");
    await expect(page.getByText("Sent to dev-whisper in Terminal")).toBeVisible();
  });

  test("a failed jump to the window is reported in the widget", async ({ page }) => {
    await open(page, "widget", {
      agents: [agent()],
      failing: { focus_waiting_agent: "Couldn't reach Terminal: none of its 2 windows mention dev-whisper." },
    });
    await page.getByTestId("agent-pill").hover();
    await page.getByTestId("agent-pill").click();
    await expect(page.getByText(/none of its 2 windows/)).toBeVisible();
  });
});

test.describe("coding agents: settings", () => {
  test.use({ viewport: { width: 880, height: 900 } });

  async function openAgents(page: Page, scenario = {}) {
    await open(page, "settings", scenario);
    await page.locator(".dw-sidebar").getByRole("button", { name: "Coding agents", exact: true }).click();
    await expect(page.locator(".dw-page-title")).toHaveText("Coding agents");
  }

  test("explains the feature and shows the exact hook config @csp", async ({ page, consoleErrors: _ }) => {
    await openAgents(page);
    const config = JSON.parse(await page.getByTestId("agent-hook-json").innerText());
    for (const event of ["Notification", "Stop", "UserPromptSubmit"]) {
      expect(config.hooks[event][0].hooks[0].command).toContain(
        "Application Support/dev.mouthpiece/bin/mouthpiece-hook",
      );
    }
    await expect(page.getByRole("button", { name: "Copy hook config" })).toBeVisible();
    await expect(page.getByTestId("agent-reply-shortcut")).toHaveText("⌃⌥R");
    await expect(page.getByTestId("agents-empty")).toBeVisible();
  });

  test("lists waiting agents and lets you dismiss or jump to one", async ({ page, consoleErrors: _ }) => {
    await openAgents(page, { agents: [agent(), agent({ session_id: "s2", folder: "api", kind: "done" })] });
    const rows = page.getByTestId("agent-row");
    await expect(rows).toHaveCount(2);
    await expect(rows.first()).toContainText("dev-whisper");
    await expect(rows.first()).toContainText("Terminal · 2 min ago");
    await expect(rows.nth(1)).toContainText("Finished");

    await rows.first().getByRole("button", { name: "Go to window" }).click();
    await expect.poll(() => calls(page, "focus_waiting_agent")).toEqual([{ sessionId: "s1" }]);

    await rows.first().getByRole("button", { name: /Dismiss/ }).click();
    await expect.poll(() => calls(page, "dismiss_waiting_agent")).toEqual([{ sessionId: "s1" }]);
    await expect(rows).toHaveCount(1);
  });

  test("the sound toggle and the reply shortcut are saved", async ({ page, consoleErrors: _ }) => {
    await openAgents(page);
    const sound = page.getByLabel("Play a soft sound when an agent starts waiting");
    await expect(sound).toBeChecked();
    await sound.uncheck();
    await expect.poll(() => calls(page, "set_agent_sound_enabled")).toEqual([{ enabled: false }]);

    await page.getByTestId("agent-reply-shortcut").click();
    await page.keyboard.press("Control+Alt+KeyJ");
    await expect.poll(() => calls(page, "set_agent_reply_shortcut")).toHaveLength(1);
  });
});
