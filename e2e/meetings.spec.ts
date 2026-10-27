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
    });
    await expect(page.getByTestId("meeting-processing")).toContainText("Transcribing · 40%");
    await emit(page, "meeting-changed", { recording: null, processing: null });
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
    await expect(rows).toHaveCount(1);
  });

  test("the number of people is saved", async ({ page, consoleErrors: _ }) => {
    await open(page, "settings");
    await goTo(page, "Meetings");
    await page.getByTestId("meeting-speakers").selectOption("3");
    expect(await calls(page, "set_meeting_speakers")).toEqual([{ speakers: 3 }]);
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

  test("shows the meeting, stops it, then reports the transcript", async ({ page, consoleErrors: _ }) => {
    await open(page, "widget");
    await page.evaluate(() => {
      const status = {
        recording: { id: "2026-10-26-174005", started_ms: Date.now() - 65_000 },
        processing: null,
      };
      window.__dw!.state.meetingStatus = status;
      return window.__dw!.emit("meeting-changed", status);
    });
    await emit(page, "recording-started");
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
    });
    await expect(page.getByText("Meeting: Transcribing 20%")).toBeVisible();
    // Dictation stays available while it's transcribed.
    await expect(page.getByRole("button", { name: "Start recording" })).toBeEnabled();

    await emit(page, "meeting-changed", { recording: null, processing: null });
    await emit(page, "meeting-saved", "Meeting, Mon 26 Oct 2026, 17:40");
    await expect(page.getByText("Meeting transcript saved")).toBeVisible();
  });
});
