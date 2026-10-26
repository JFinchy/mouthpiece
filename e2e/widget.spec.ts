import type { Page } from "@playwright/test";
import { calls, emit, expect, open, test } from "./fixtures";

test.use({ viewport: { width: 280, height: 360 } });

// Hovering opens the flyout, which in a tall browser viewport moves the
// vertically-centered record button (the real window grows downward
// instead). Settle the hover first so the click lands.
async function clickRecord(page: Page) {
  await page.locator("main").hover();
  await page.getByRole("button", { name: "Start recording" }).click();
}

test.describe("widget", () => {
  test("idle widget is ready and names the mic", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await expect(page.getByText("Ready")).toBeVisible();
    await expect(page.getByRole("button", { name: "Start recording" })).toHaveAttribute(
      "title",
      /Mic: MacBook Pro Microphone/,
    );
  });

  test("a full dictation walks through every phase @csp", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await clickRecord(page);
    await expect.poll(() => calls(page, "toggle_recording_command")).toHaveLength(1);

    await emit(page, "recording-started");
    await expect(page.getByTestId("level-meter")).toBeVisible();
    await emit(page, "audio-level", 0.8);

    await emit(page, "recording-stopped", "/tmp/x.wav");
    await expect(page.getByRole("button", { name: "Stop recording" })).toBeDisabled();
    await expect(page.getByText("Transcribing…")).toBeVisible();

    await emit(page, "refining-started", "qwen3.5:4b");
    await expect(page.getByRole("progressbar", { name: "Refining" })).toBeVisible();

    await emit(page, "transcript-ready", "Ship it.");
    await expect(page.getByText("Pasted")).toBeVisible();
    await expect(page.getByText("Ready")).toBeVisible({ timeout: 4000 });
  });

  test("a silent mic is called out while recording and clears when sound returns", async ({
    page,
    consoleErrors: _,
  }) => {
    await open(page, "widget");
    await emit(page, "recording-started");
    await emit(page, "mic-silent", "No sound from Microsoft Teams Audio. Check it's on and unmuted.");
    await expect(page.getByText("No sound from mic")).toBeVisible();
    await expect(page.getByTestId("level-meter")).toHaveCount(0);

    await emit(page, "mic-silent-cleared");
    await expect(page.getByText("No sound from mic")).toHaveCount(0);
    await expect(page.getByTestId("level-meter")).toBeVisible();
  });

  test("errors from the pipeline are shown, then fade back to ready", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await emit(page, "recording-started");
    await emit(page, "recording-stopped", "/tmp/x.wav");
    await emit(page, "transcript-error", "no speech detected");
    await expect(page.getByText("no speech detected")).toBeVisible();
  });

  test("hovering lists every mic and switching one updates the highlight @csp", async ({
    page,
    consoleErrors: _,
  }) => {
    await open(page, "widget");
    await page.locator("main").hover();
    await expect(page.getByText("Microphone", { exact: true })).toBeVisible();
    const airhug = page.getByRole("button", { name: "AIRHUG 28" });
    await expect(airhug).not.toHaveClass(/\bsel\b/);

    await airhug.click();
    await expect.poll(() => calls(page, "set_input_device")).toEqual([{ name: "AIRHUG 28" }]);
    await expect(airhug).toHaveClass(/\bsel\b/);
  });

  test("a missing preferred mic is visible, not silent", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget", {
      devices: ["MacBook Pro Microphone"],
      deviceStatus: { kind: "fallback", wanted: "AIRHUG 28", using: "MacBook Pro Microphone" },
    });
    await expect(page.getByText("Using MacBook Pro Microphone")).toBeVisible();
    await page.locator("main").hover();
    await expect(page.getByRole("button", { name: "AIRHUG 28 (not connected)" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "MacBook Pro Microphone" })).toHaveClass(/\bsel\b/);
  });

  test("a fresh install says what's missing and opens Settings", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget", {
      readiness: [
        {
          id: "model",
          level: "error",
          title: "Download a speech model",
          detail: "Base (English) isn't downloaded yet.",
          fix: "download_model",
        },
      ],
    });
    const setup = page.getByTestId("widget-setup");
    await expect(setup).toHaveText("Finish setup");
    await expect(setup).toHaveAttribute("title", /Download a speech model/);
    await page.locator("main").hover();
    await setup.click();
    await expect.poll(() => calls(page, "open_settings")).toHaveLength(1);
  });

  test("a rejected action is reported in the widget", async ({ page }) => {
    await open(page, "widget", { failing: { toggle_recording_command: "mic busy" } });
    await clickRecord(page);
    await expect(page.getByText("Couldn't start recording: mic busy")).toBeVisible();
  });

  test("detailed mode explains a blocking setup problem", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget", {
      widgetMode: "detailed",
      readiness: [
        {
          id: "paste",
          level: "error",
          title: "Allow pasting",
          detail: "Mouthpiece needs Accessibility permission.",
          fix: "grant_accessibility",
        },
      ],
    });
    await expect(page.getByText("Allow pasting. Mouthpiece needs Accessibility permission.")).toBeVisible();
  });

  test("transcription shows a percentage and a filling bar", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await emit(page, "recording-started");
    await emit(page, "recording-stopped", "/tmp/x.wav");
    await emit(page, "transcribe-progress", 37);
    const bar = page.getByRole("progressbar", { name: "Transcribing" });
    await expect(bar).toHaveAttribute("aria-valuenow", "37");
    await expect(bar).toContainText("37%");
    await emit(page, "transcribe-progress", 82);
    await expect(bar).toHaveAttribute("aria-valuenow", "82");

    await emit(page, "transcript-ready", "Ship it.");
    await expect(bar).toHaveCount(0);
  });

  test("minimal mode fills a ring while transcribing", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget", { widgetMode: "minimal" });
    await emit(page, "recording-started");
    await emit(page, "recording-stopped", "/tmp/x.wav");
    await emit(page, "transcribe-progress", 50);
    await expect(page.getByTestId("transcribe-progress-ring")).toBeVisible();
  });

  test("recording shows elapsed time", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await emit(page, "recording-started");
    await expect(page.getByTestId("recording-elapsed")).toHaveText("0:00");
    await expect(page.getByTestId("recording-elapsed")).toHaveText("0:01", { timeout: 3000 });
  });

  test("detailed mode shows the live transcript while recording and clears it on stop", async ({
    page,
    consoleErrors: _,
  }) => {
    await open(page, "widget", { widgetMode: "detailed" });
    await emit(page, "recording-started");
    await expect(page.getByTestId("live-transcript")).toHaveCount(0);

    await emit(page, "live-transcript", "Refactor the auth middleware.");
    await expect(page.getByTestId("live-transcript")).toHaveText("Refactor the auth middleware.");

    await emit(page, "live-transcript", "Refactor the auth middleware. Then run the migration.");
    await expect(page.getByTestId("live-transcript")).toContainText("Then run the migration.");

    await emit(page, "recording-stopped", "/tmp/x.wav");
    await expect(page.getByTestId("live-transcript")).toHaveCount(0);
  });

  test("compact mode keeps the live transcript out of the small widget", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await emit(page, "recording-started");
    await emit(page, "live-transcript", "Refactor the auth middleware.");
    await expect(page.getByTestId("level-meter")).toBeVisible();
    await expect(page.getByText("Refactor the auth middleware.")).toHaveCount(0);
  });

  test("detailed mode names the real hotkey", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget", { widgetMode: "detailed" });
    await expect(page.getByText(/Press ⇧⌘Space or click the button to start/)).toBeVisible();
  });

  test("LLM refining is its own stage: violet, sparkle, its own percentage", async ({
    page,
    consoleErrors: _,
  }) => {
    await open(page, "widget");
    await emit(page, "recording-started");
    await emit(page, "recording-stopped", "/tmp/x.wav");
    await emit(page, "transcribe-progress", 100);
    const transcribe = page.getByRole("progressbar", { name: "Transcribing" });
    await expect(transcribe).toBeVisible();

    await emit(page, "refining-started", "qwen3.5:4b");
    await expect(transcribe).toHaveCount(0);
    const refine = page.getByRole("progressbar", { name: "Refining" });
    await expect(refine).toHaveAttribute("title", /^Cleaning up with qwen3\.5:4b\./);
    await emit(page, "refine-progress", 40);
    await expect(refine).toHaveAttribute("aria-valuenow", "40");
    await expect(refine).toContainText("40%");
    // Violet (--dw-refine), not the amber transcribe accent.
    await expect(refine.locator(".dw-shimmer")).toHaveCSS("background-color", "rgb(167, 139, 250)");

    await emit(page, "transcript-ready", "Done.");
    await expect(refine).toHaveCount(0);
  });

  test("boilerplate refining without progress shows an indeterminate bar", async ({
    page,
    consoleErrors: _,
  }) => {
    await open(page, "widget");
    await emit(page, "recording-started");
    await emit(page, "recording-stopped", "/tmp/x.wav");
    await emit(page, "refining-started", "qwen3.5:4b");
    const refine = page.getByRole("progressbar", { name: "Refining" });
    await expect(refine).toBeVisible();
    await expect(refine).not.toContainText("%");
  });

  test("clicking the refining bar pastes as-is", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await emit(page, "recording-started");
    await emit(page, "recording-stopped", "/tmp/x.wav");
    await emit(page, "refining-started", "qwen3.5:4b");
    const refine = page.getByRole("progressbar", { name: "Refining" });
    await expect(refine).toHaveAttribute("title", /Click to paste as-is \(⌥⇧⌘Space\)/);
    await page.locator("main").hover();
    await refine.click();
    await expect.poll(() => calls(page, "paste_as_is_command")).toHaveLength(1);
  });

  test("a recording can be discarded and brought back", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await emit(page, "recording-started");
    // Entering the widget opens its hover menu, which moves the row.
    await page.getByTestId("discard").hover();
    await page.getByTestId("discard").click();
    await expect.poll(() => calls(page, "discard_recording")).toHaveLength(1);

    await emit(page, "dictation-discarded", "recording");
    await expect(page.getByText("Recording discarded")).toBeVisible();
    await expect(page.getByTestId("discard")).toHaveCount(0);
    await page.getByTestId("undo-discard").hover();
    await page.getByTestId("undo-discard").click();
    await expect.poll(() => calls(page, "undo_discard")).toHaveLength(1);
    await expect(page.getByTestId("undo-discard")).toHaveCount(0);
  });

  test("the undo offer goes away when it expires", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await emit(page, "dictation-discarded", "cancelled");
    await expect(page.getByText("Paste cancelled")).toBeVisible();
    await expect(page.getByTestId("undo-discard")).toBeVisible();
    await emit(page, "discard-expired");
    await expect(page.getByTestId("undo-discard")).toHaveCount(0);
  });

  test("the hover menu switches the widget style", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await page.locator("main").hover();
    const picker = page.getByTestId("style-picker");
    await expect(picker.getByRole("button", { name: "Compact" })).toHaveAttribute("aria-pressed", "true");
    await picker.getByRole("button", { name: "Notch" }).click();
    await expect.poll(() => calls(page, "set_widget_mode")).toEqual([{ mode: "notch" }]);
  });

  for (const style of ["pill", "notch", "caption", "stacked"] as const) {
    test(`the ${style} style records, shows progress and offers discard`, async ({ page, consoleErrors: _ }) => {
      await open(page, "widget", { widgetMode: style });
      await emit(page, "recording-started");
      await expect(page.getByTestId("discard")).toBeVisible();
      await emit(page, "recording-stopped");
      await emit(page, "transcribe-progress", 40);
      await expect(page.getByTestId("transcribe-progress")).toBeVisible();
      await emit(page, "transcript-ready", "Look at @recording.rs");
      await expect(page.getByTestId("discard")).toHaveCount(0);
    });
  }

  test("live caption shows your words, then what was pasted", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget", { widgetMode: "caption" });
    await emit(page, "recording-started");
    await emit(page, "live-transcript", "fix the stop path");
    await expect(page.getByTestId("live-transcript")).toContainText("fix the stop path");
    await emit(page, "recording-stopped");
    await emit(page, "transcript-ready", "Fix the stop path.");
    await expect(page.getByTestId("last-pasted")).toHaveText("Fix the stop path.");
  });
});
