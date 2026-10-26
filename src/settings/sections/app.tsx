import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { check as checkForUpdate, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { THEME_ORDER, THEME_LABEL, THEMES, type ThemeId } from "../../theme";
import { notifyError } from "../../ui/toast";
import { formatTimestamp } from "./history";

export type LogEntry = { timestamp_ms: number; message: string };

/// Collapsed by default — this is a diagnostic tool ("why didn't it
/// paste?", "which mode did it pick?"), not something most users need
/// open, and polling only while expanded avoids invoke() calls for a
/// panel nobody's looking at.
export function LogsSection() {
  const [open, setOpen] = useState(false);
  const [logs, setLogs] = useState<LogEntry[]>([]);

  function refresh() {
    invoke<LogEntry[]>("get_logs")
      .then(setLogs)
      .catch((err) => notifyError("get_logs", err));
  }

  useEffect(() => {
    if (!open) return;
    refresh();
    const interval = setInterval(refresh, 2000);
    return () => clearInterval(interval);
  }, [open]);

  function clear() {
    setLogs([]);
    invoke("clear_logs").catch((err) => notifyError("clear_logs", err));
  }

  return (
    <div className="mb-4 border-t border-base-content/10 pt-3">
      <div className="mb-1 flex items-center justify-between">
        <button
          className="text-xs font-medium opacity-70"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
        >
          {open ? "▾" : "▸"} Logs
        </button>
        {open && (
          <div className="flex items-center gap-1.5">
            <button className="btn btn-ghost btn-xs" onClick={refresh}>
              Refresh
            </button>
            <button className="btn btn-ghost btn-xs" onClick={clear} disabled={logs.length === 0}>
              Clear
            </button>
          </div>
        )}
      </div>
      {open &&
        (logs.length === 0 ? (
          <p className="text-xs opacity-60">No log entries yet.</p>
        ) : (
          <pre className="max-h-48 overflow-y-auto whitespace-pre-wrap rounded-md bg-base-100 p-1.5 font-mono text-[10px] leading-tight">
            {logs.map((l) => `[${formatTimestamp(l.timestamp_ms)}] ${l.message}`).join("\n")}
          </pre>
        ))}
    </div>
  );
}

export function ThemeSection({ theme, onChange }: { theme: ThemeId; onChange: (id: ThemeId) => void }) {
  return (
    <div className="mb-4 border-b border-base-content/10 pb-3">
      <label className="mb-1 block text-xs font-medium opacity-70">Appearance</label>
      <div className="flex flex-col gap-1.5">
        {THEME_ORDER.map((id) => (
          <button
            key={id}
            type="button"
            className={`dw-theme-row ${theme === id ? "active" : ""}`}
            onClick={() => onChange(id)}
          >
            <span className="flex items-center gap-2">
              <span className="dw-theme-dot" style={{ background: THEMES[id].accent }} />
              {THEME_LABEL[id]}
            </span>
            {theme === id && <span>✓</span>}
          </button>
        ))}
      </div>
    </div>
  );
}

export type LayoutId = "sidebar" | "chain";

export const LAYOUT_LABEL: Record<LayoutId, string> = { sidebar: "Signal", chain: "Signal Chain" };

export const LAYOUT_DESCRIPTION: Record<LayoutId, string> = {
  sidebar: "Left sidebar, one page of content at a time",
  chain: "Horizontal pipeline — click a stage to expand it",
};

export function LayoutSection({ layout, onChange }: { layout: LayoutId; onChange: (id: LayoutId) => void }) {
  return (
    <div className="mb-4">
      <label className="mb-1 block text-xs font-medium opacity-70">Layout</label>
      <div className="flex flex-col gap-1.5">
        {(["sidebar", "chain"] as LayoutId[]).map((id) => (
          <button
            key={id}
            type="button"
            className={`dw-theme-row ${layout === id ? "active" : ""}`}
            onClick={() => onChange(id)}
          >
            <span className="flex flex-col items-start gap-0.5">
              <span>{LAYOUT_LABEL[id]}</span>
              <span className="text-[10px] font-normal opacity-60">{LAYOUT_DESCRIPTION[id]}</span>
            </span>
            {layout === id && <span>✓</span>}
          </button>
        ))}
      </div>
    </div>
  );
}

export const ABOUT_ENTRIES: { title: string; body: string }[] = [
  {
    title: "How dictation works",
    body: "Hold the push-to-talk shortcut (or click-and-hold the widget) and speak. When you release, Mouthpiece transcribes locally with Whisper, applies your active Mode's formatting, and pastes the result into whatever's focused — no cloud round-trip unless you've turned on LLM refinement or a webhook.",
  },
  {
    title: "Voice Isolation",
    body: "Filters recordings down to your voice before transcription, so background noise or other people talking don't end up in the transcript. Works out of the box with an energy-based fallback; enroll your voice in Voice for stronger speaker-based isolation.",
  },
  {
    title: "Vocabulary",
    body: "Terms you add bias Whisper's recognition toward your jargon — tool names, library names, anything Whisper would otherwise mishear. Vocabulary books let you adopt a curated bundle (Frontend, Backend, Full-Stack, Product) in one click, then prune anything you don't need.",
  },
  {
    title: "Snippets",
    body: "Spoken trigger phrases that expand into saved blocks of text — say a snippet's trigger and it's pasted in full, instead of dictating the same boilerplate every time.",
  },
  {
    title: "Modes",
    body: 'Plain, Casual, or CLI formatting, applied automatically based on which app is frontmost, or overridden for a single dictation from the widget\'s hover flyout. CLI mode turns "git commit update readme" into git commit -m "update readme", for example.',
  },
  {
    title: "LLM refinement",
    body: "An optional local pass through Ollama that cleans up a transcript further — only runs for modes with it explicitly enabled, since it's an extra background call.",
  },
  {
    title: "History",
    body: "Every delivered transcript is logged, searchable, and auto-purged after your configured retention window. Turn on the work journal to get a one-line LLM-generated summary attached to each entry.",
  },
  {
    title: "Integrations",
    body: "Copy-only mode skips the simulated paste keystroke entirely (and the Accessibility permission it needs). The output webhook fires a POST with the delivered transcript to any URL that accepts incoming webhooks — Notion, Slack, n8n, Zapier, Make.com, or your own endpoint.",
  },
];

export function AboutSection() {
  return (
    <div>
      {ABOUT_ENTRIES.map((entry) => (
        <div key={entry.title} className="dw-about-entry">
          <h3>{entry.title}</h3>
          <p>{entry.body}</p>
        </div>
      ))}
    </div>
  );
}

export type UpdateCheckState = "checking" | "idle" | "available" | "downloading" | "error";

export function UpdatesSection() {
  const [version, setVersion] = useState("");
  const [state, setState] = useState<UpdateCheckState>("checking");
  const [update, setUpdate] = useState<Update | null>(null);
  const [percent, setPercent] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  function runCheck() {
    setState("checking");
    setError(null);
    checkForUpdate()
      .then((found) => {
        setUpdate(found);
        setState(found ? "available" : "idle");
      })
      .catch((err) => {
        notifyError("checkForUpdate", err);
        setError(String(err));
        setState("error");
      });
  }

  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch((err) => notifyError("getVersion", err));
    runCheck();
  }, []);

  function installUpdate() {
    if (!update) return;
    setState("downloading");
    setPercent(0);
    // contentLength isn't guaranteed by every server response, so a null
    // percent (rather than a wrong one) is the honest fallback — the
    // "Downloading…" label alone still shows real progress is happening.
    let total = 0;
    let downloaded = 0;
    update
      .downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? 0;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          setPercent(total > 0 ? Math.round((downloaded / total) * 100) : null);
        }
      })
      .then(() => relaunch())
      .catch((err) => {
        notifyError("downloadAndInstall", err);
        setError(String(err));
        setState("error");
      });
  }

  return (
    <div className="rounded-lg border border-base-content/10 bg-base-100 p-4">
      <h3 className="mb-3 text-xs font-semibold uppercase tracking-wide opacity-60">Updates</h3>
      <div className="flex items-center justify-between gap-2 text-xs">
        <span className="opacity-70">Version {version || "…"}</span>
        {state === "checking" && <span className="loading loading-spinner loading-xs" />}
        {state === "idle" && (
          <button className="btn btn-ghost btn-xs" onClick={runCheck}>
            Check for updates
          </button>
        )}
        {state === "available" && update && (
          <button className="btn btn-xs" onClick={installUpdate}>
            Update to v{update.version}
          </button>
        )}
        {state === "downloading" && (
          <span className="opacity-70">{percent !== null ? `Downloading… ${percent}%` : "Downloading…"}</span>
        )}
        {state === "error" && (
          <button className="btn btn-ghost btn-xs" onClick={runCheck}>
            Retry
          </button>
        )}
      </div>
      {error && <p className="mt-1.5 text-xs text-error">{error}</p>}
      <p className="mt-1.5 text-xs opacity-50">
        Checks GitHub on launch and whenever this page opens — no data sent, just "is there a newer release."
      </p>
    </div>
  );
}
