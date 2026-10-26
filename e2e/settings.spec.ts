import { calls, expect, open, test } from "./fixtures";

test.use({ viewport: { width: 880, height: 720 } });

const PAGES = [
  ["Dictation", "Dictation"],
  ["Voice", "Voice — Whisper model & isolation"],
  ["Vocabulary", "Vocabulary"],
  ["Modes", "Modes — app-aware formatting rules"],
  ["LLM", "LLM — local refinement (Ollama)"],
  ["Insights", "Insights — usage & feature adoption"],
  ["History", "History"],
  ["Integrations", "Integrations — delivery & webhook"],
  ["Coding agents", "Coding agents"],
  ["Appearance", "Appearance"],
  ["About", "About — features & how it works"],
  ["Advanced", "Advanced"],
] as const;

test.describe("settings", () => {
  test("every page renders without errors @csp", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings");
    for (const [nav, title] of PAGES) {
      await page.locator(".dw-sidebar").getByRole("button", { name: nav, exact: true }).click();
      await expect(page.locator(".dw-page-title")).toHaveText(title);
    }
    await expect(page.getByTestId("toast")).toHaveCount(0);
  });

  test("Transcribe while you talk is on by default and can be switched off", async ({
    page,
    consoleErrors: _,
  }) => {
    await open(page, "settings");
    await page.locator(".dw-sidebar").getByRole("button", { name: "Voice", exact: true }).click();
    const toggle = page.getByRole("checkbox", { name: "Transcribe while you talk" });
    await expect(toggle).toBeChecked();

    await toggle.uncheck();
    await expect.poll(() => calls(page, "set_live_transcribe_enabled")).toEqual([{ enabled: false }]);
    await expect(toggle).not.toBeChecked();
  });

  test("the setup checklist stays hidden when everything is ready", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings");
    await expect(page.locator(".dw-page-title")).toHaveText("Dictation");
    await expect(page.getByTestId("readiness-panel")).toHaveCount(0);
  });

  test("a missing model leads straight to the model picker", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings", {
      readiness: [
        { id: "model", level: "error", title: "Download a speech model", detail: "Base (English) isn't downloaded yet.", fix: "download_model" },
        { id: "paste", level: "ok", title: "Pasting allowed", detail: "", fix: null },
      ],
    });
    const panel = page.getByTestId("readiness-panel");
    await expect(panel.getByRole("heading")).toHaveText("Finish setup to start dictating");
    await expect(panel.getByTestId("readiness-paste")).toHaveCount(0);

    await panel.getByRole("button", { name: "Choose a model" }).click();
    await expect(page.locator(".dw-page-title")).toHaveText("Voice — Whisper model & isolation");
  });

  test("the Accessibility fix opens System Settings", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings", {
      readiness: [{ id: "paste", level: "error", title: "Allow pasting", detail: "Needs Accessibility.", fix: "grant_accessibility" }],
    });
    await page.getByRole("button", { name: "Open System Settings" }).click();
    await expect.poll(() => calls(page, "open_accessibility_settings")).toHaveLength(1);
  });

  test("the microphone fixes ask macOS or open its settings", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings", {
      readiness: [
        { id: "mic_access", level: "warn", title: "Allow the microphone", detail: "macOS asks once.", fix: "allow_microphone" },
      ],
    });
    await page.getByRole("button", { name: "Allow microphone" }).click();
    await expect.poll(() => calls(page, "request_microphone_access")).toHaveLength(1);

    await open(page, "settings", {
      readiness: [
        { id: "mic_access", level: "error", title: "Microphone access is off", detail: "Turn it on.", fix: "open_microphone_settings" },
      ],
    });
    await page.getByRole("button", { name: "Open System Settings" }).click();
    await expect.poll(() => calls(page, "open_microphone_settings")).toHaveLength(1);
  });

  test("warnings get a softer heading than blockers", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings", {
      readiness: [{ id: "llm", level: "warn", title: "Ollama isn't running", detail: "No cleanup until it is.", fix: "start_ollama" }],
    });
    await expect(page.getByTestId("readiness-panel").getByRole("heading")).toHaveText("Heads up");
  });

  test("a failed save shows a toast instead of failing silently", async ({ page }) => {
    await open(page, "settings", { failing: { set_vocabulary: "disk full" } });
    await page.locator(".dw-sidebar").getByRole("button", { name: "Vocabulary", exact: true }).click();
    await page.getByPlaceholder("Add a term (e.g. kubectl)").fill("Hammerspoon");
    await page.getByRole("button", { name: "Add", exact: true }).first().click();
    const toast = page.getByTestId("toast");
    await expect(toast).toContainText("Couldn't save vocabulary");
    await expect(toast).toContainText("disk full");
  });

  test("project names can be turned off and show the detected project", async ({ page }) => {
    await open(page, "settings", { projectStatus: { name: "dev-whisper", names: 1240 } });
    await page.locator(".dw-sidebar").getByRole("button", { name: "Vocabulary", exact: true }).click();
    const toggle = page.getByLabel("Use names from your project");
    await expect(toggle).toBeChecked();
    await expect(page.getByTestId("project-status")).toHaveText("Last project: dev-whisper · 1,240 names");
    await toggle.uncheck();
    await expect.poll(() => calls(page, "set_project_names_enabled")).toEqual([{ enabled: false }]);
    await expect(page.getByTestId("project-status")).toHaveCount(0);
  });

  test("project names say so when no project has been found yet", async ({ page }) => {
    await open(page, "settings");
    await page.locator(".dw-sidebar").getByRole("button", { name: "Vocabulary", exact: true }).click();
    await expect(page.getByTestId("project-status")).toContainText("No project yet");
  });

  test("insights show the typical time to paste @csp", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings");
    await page.locator(".dw-sidebar").getByRole("button", { name: "Insights", exact: true }).click();
    await expect(page.getByText("Time to paste")).toBeVisible();
    await expect(page.getByText("640ms")).toBeVisible();
    await expect(page.getByText(/Slowest 1 in 10: 1\.9s/)).toBeVisible();
  });

  test("the chain layout shows the setup checklist too", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings", {
      layout: "chain",
      readiness: [{ id: "hotkey", level: "error", title: "Pick a different hotkey", detail: "Taken.", fix: "change_shortcut" }],
    });
    await expect(page.getByText("Mouthpiece — signal chain")).toBeVisible();
    await expect(page.getByTestId("readiness-hotkey")).toBeVisible();
  });

  // Regression: daisyUI surfaces followed the OS appearance while the app
  // theme set the text color, so light-mode Macs got light text on white
  // cards under the dark Terminal theme.
  for (const scheme of ["light", "dark"] as const) {
    test(`cards follow the app theme with the OS in ${scheme} mode @csp`, async ({ browser }) => {
      const page = await browser.newPage({ colorScheme: scheme, viewport: { width: 880, height: 720 } });
      await open(page, "settings");
      await page.locator(".dw-sidebar").getByRole("button", { name: "Voice", exact: true }).click();
      const card = page.locator(".bg-base-100").first();
      await expect(card).toBeVisible();
      // Terminal's card color, #1c1f23.
      await expect(card).toHaveCSS("background-color", "rgb(28, 31, 35)");
      await page.close();
    });
  }

  test("refine-only-when-needed is on by default and can be turned off", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings");
    await page.locator(".dw-sidebar").getByRole("button", { name: "LLM", exact: true }).click();
    const toggle = page.getByRole("checkbox", { name: "Only refine when it's needed (faster)" });
    await expect(toggle).toBeChecked();
    await toggle.uncheck();
    await expect.poll(() => calls(page, "set_llm_only_when_needed")).toEqual([{ enabled: false }]);
  });

  test("the paste-as-is shortcut is the hotkey plus Shift (or Option)", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings");
    // The mock hotkey is ⇧⌘Space, which already has Shift, so Option is added.
    await expect(page.getByTestId("skip-shortcut-hint")).toContainText("⌥⇧⌘Space stops and pastes as-is");
  });

  test("the widget style picker shows every style and switches", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings");
    const picker = page.getByRole("radiogroup", { name: "Widget style" });
    await expect(picker.getByRole("radio")).toHaveCount(7);
    await expect(page.getByTestId("style-compact")).toHaveAttribute("aria-checked", "true");
    await page.getByTestId("style-caption").click();
    await expect.poll(() => calls(page, "set_widget_mode")).toEqual([{ mode: "caption" }]);
    await expect(page.getByTestId("style-caption")).toHaveAttribute("aria-checked", "true");
  });
});
