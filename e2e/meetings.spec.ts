import type { Page } from "@playwright/test";
import { calls, emit, expect, open, test } from "./fixtures";

async function goTo(page: Page, nav: string) {
  await page.locator(".dw-sidebar").getByRole("button", { name: nav, exact: true }).click();
}

test.describe("meetings page", () => {
  test.use({ viewport: { width: 880, height: 720 } });

  test("start, stop, then the transcript is being made @csp", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings");
    await goTo(page, "Meetings");
    await page.getByTestId("meeting-toggle").click();
    await expect(page.getByTestId("meeting-elapsed")).toContainText("Recording · 0:0");
    await expect(page.getByTestId("meeting-toggle")).toHaveText("Stop meeting");

    await page.getByTestId("meeting-toggle").click();
    await expect(page.getByTestId("meeting-processing")).toContainText("Finding who spoke");
    await emit(page, "meeting-changed", {
      recording: null,
      processing: { id: "2026-10-26-174005", step: "Transcribing", percent: 40 },
      queued: ["2026-10-26-174005"],
    });
    await expect(page.getByTestId("meeting-processing")).toContainText("Transcribing · 40%");
    await emit(page, "meeting-changed", { recording: null, processing: null, queued: [] });
    await expect(page.getByTestId("meeting-toggle")).toHaveText("Start meeting");
  });

  test("lists transcripts: view, retry, delete", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings");
    await goTo(page, "Meetings");
    const rows = page.getByTestId("meeting-row");
    await expect(rows).toHaveCount(2);
    await expect(rows.first()).toContainText("24 min · 3 speakers");

    await rows.first().getByRole("button", { name: "View" }).click();
    await expect(rows.first().locator("pre")).toContainText("Let's get started.");

    await expect(rows.nth(1)).toContainText("Not transcribed yet");
    await rows.nth(1).getByRole("button", { name: "Transcribe" }).click();
    expect(await calls(page, "transcribe_meeting")).toEqual([{ id: "2026-10-25-161500" }]);

    await rows.first().hover();
    await rows.first().getByRole("button", { name: /^Delete/ }).click();
    await expect(rows).toHaveCount(2);
    await rows.first().getByRole("button", { name: /^Confirm deleting/ }).click();
    await expect(rows).toHaveCount(1);
  });

  test("the number of people and auto-detect setting are saved", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings");
    await goTo(page, "Meetings");
    await page.getByTestId("meeting-speakers").selectOption("3");
    expect(await calls(page, "set_meeting_speakers")).toEqual([{ speakers: 3 }]);

    const autoDetect = page.getByTestId("meeting-auto-detect");
    await expect(autoDetect).toBeChecked();
    await autoDetect.uncheck();
    expect(await calls(page, "set_meeting_auto_detect")).toEqual([{ enabled: false }]);
  });

  test("transcripts stay hidden behind the History lock", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings", { historyLock: true });
    await goTo(page, "Meetings");
    await expect(page.getByTestId("history-locked")).toBeVisible();
    await expect(page.getByTestId("meeting-row")).toHaveCount(0);
    expect(await calls(page, "list_meetings")).toHaveLength(0);
    // Recording doesn't need unlocking.
    await expect(page.getByTestId("meeting-toggle")).toBeEnabled();

    await page.getByRole("button", { name: "Unlock History" }).click();
    await expect(page.getByTestId("meeting-row")).toHaveCount(2);
  });
});

test.describe("meeting in the widget", () => {
  test.use({ viewport: { width: 280, height: 360 } });

  test("goes back to idle when the meeting ends, even without the stop event", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await emit(page, "meeting-changed", {
      recording: { id: "2026-10-26-174005", started_ms: Date.now() },
      processing: null,
      queued: [],
    });
    await expect(page.getByTestId("recording-elapsed")).toContainText("Meeting 0:0");
    await emit(page, "meeting-blocked", "Dictation is paused while a meeting records");
    await expect(page.getByTestId("recording-elapsed")).toContainText("Dictation is paused");
    await emit(page, "meeting-changed", { recording: null, processing: null, queued: [] });
    await expect(page.getByText("Ready")).toBeVisible();
  });

  test("counts down to the 2.5-hour limit in the last 10 minutes", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    const limit = 150 * 60_000;
    await emit(page, "meeting-changed", {
      recording: { id: "2026-10-26-174005", started_ms: Date.now() - (limit - 5 * 60_000), limit_ms: limit },
      processing: null,
      queued: [],
    });
    await expect(page.getByTestId("recording-elapsed")).toContainText("Ends in 4:5");
    await expect(page.getByTestId("recording-elapsed")).toHaveClass(/text-warning/);
    // The backend stops the meeting, then says why.
    await emit(page, "meeting-stopped");
    await emit(page, "meeting-changed", {
      recording: null,
      processing: { id: "2026-10-26-174005", step: "Finding who spoke", percent: null },
      queued: ["2026-10-26-174005"],
    });
    await emit(page, "meeting-limit-reached");
    await expect(page.getByText("Meeting stopped at the 2.5-hour limit")).toBeVisible();
  });

  test("the main button stops a push-to-talk dictation, not the meeting", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await emit(page, "meeting-changed", {
      recording: { id: "2026-10-26-174005", started_ms: Date.now() },
      processing: null,
      queued: [],
    });
    await emit(page, "recording-started", "meeting");
    await emit(page, "recording-started"); // push-to-talk inside the meeting
    await page.locator("main").hover();
    await page.getByRole("button", { name: "Stop recording" }).click();
    expect(await calls(page, "toggle_recording_command")).toHaveLength(1);
    expect(await calls(page, "stop_meeting")).toHaveLength(0);
    // The meeting stopping mid-dictation leaves the dictation recording.
    await emit(page, "meeting-stopped");
    await emit(page, "meeting-changed", { recording: null, processing: null, queued: [] });
    await expect(page.getByRole("button", { name: "Stop recording" })).toBeVisible();
  });

  test("shows the meeting, stops it, then reports the transcript", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await page.evaluate(() => {
      const status = {
        recording: { id: "2026-10-26-174005", started_ms: Date.now() - 65_000 },
        processing: null,
        queued: [],
      };
      window.__dw!.state.meetingStatus = status;
      return window.__dw!.emit("meeting-changed", status);
    });
    await emit(page, "recording-started", "meeting");
    await expect(page.getByTestId("recording-elapsed")).toContainText("Meeting 1:0");
    await expect(page.getByTestId("discard")).toHaveCount(0);

    await page.locator("main").hover();
    await page.getByRole("button", { name: "Stop the meeting" }).click();
    expect(await calls(page, "stop_meeting")).toHaveLength(1);
    expect(await calls(page, "toggle_recording_command")).toHaveLength(0);
    await page.mouse.move(0, 350);

    await emit(page, "meeting-stopped");
    await emit(page, "meeting-changed", {
      recording: null,
      processing: { id: "2026-10-26-174005", step: "Transcribing", percent: 20 },
      queued: ["2026-10-26-174005"],
    });
    await expect(page.getByText("Meeting: Transcribing 20%")).toBeVisible();
    // Dictation stays available while it's transcribed.
    await expect(page.getByRole("button", { name: "Start recording" })).toBeEnabled();

    await emit(page, "meeting-changed", { recording: null, processing: null, queued: [] });
    await emit(page, "meeting-saved", "Meeting, Mon 26 Oct 2026, 17:40");
    await expect(page.getByText("Meeting transcript saved")).toBeVisible();
  });

  test("prompts to record a detected call, allows dismissal, and prompts when the call ends", async ({
    page,
    consoleErrors: _,
  }) => {
    await open(page, "widget");
    const call = {
      platform: "google_meet",
      app_name: "Google Meet",
      bundle_id: "com.google.Chrome",
      pid: 4242,
    };
    await emit(page, "meeting-detected", call);
    await expect(page.getByTestId("meeting-detect-start")).toHaveText("Record Google Meet?");

    await page.getByTestId("meeting-detect-dismiss").click();
    expect(await calls(page, "dismiss_meeting_prompt")).toHaveLength(1);
    await expect(page.getByTestId("meeting-detect-prompt")).toHaveCount(0);

    // Re-emit a new call and click "Record Zoom?" to start recording.
    await emit(page, "meeting-detected", {
      platform: "zoom",
      app_name: "Zoom",
      bundle_id: "us.zoom.xos",
      pid: 5151,
    });
    await page.getByTestId("meeting-detect-start").click();
    expect(await calls(page, "start_meeting")).toHaveLength(1);
    await expect(page.getByTestId("recording-elapsed")).toContainText("Meeting 0:0");

    // When the call ends, the widget offers a one-click stop prompt.
    await emit(page, "meeting-ended-detected", {
      platform: "zoom",
      app_name: "Zoom",
      bundle_id: "us.zoom.xos",
      pid: 5151,
    });
    await expect(page.getByTestId("meeting-ended-prompt")).toHaveText("Call ended · Stop?");
    await page.getByTestId("meeting-ended-prompt").click();
    expect(await calls(page, "stop_meeting")).toHaveLength(1);
  });
});
