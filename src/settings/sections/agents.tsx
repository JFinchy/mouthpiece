import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { notifyError } from "../../ui/toast";
import { shortcutLabel, type ShortcutConfig } from "../../shortcut";
import { IGNORED_KEYS } from "./dictation";

// Mirrors `agents::WaitingAgent`.
export type WaitingAgent = {
  session_id: string;
  cwd: string;
  folder: string;
  message: string;
  kind: "needs_input" | "done";
  app_pid: number | null;
  bundle_id: string | null;
  app_name: string | null;
  tty: string | null;
  received_ms: number;
};

type HookConfig = { script_path: string; json: string };

export function ageLabel(receivedMs: number, now = Date.now()): string {
  const minutes = Math.floor((now - receivedMs) / 60_000);
  return minutes < 1 ? "just now" : `${minutes} min ago`;
}

function ReplyShortcut() {
  const [shortcut, setShortcut] = useState<ShortcutConfig | null>(null);
  const [registerError, setRegisterError] = useState<string | null>(null);
  const [listening, setListening] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<ShortcutConfig>("get_agent_reply_shortcut")
      .then(setShortcut)
      .catch((err) => notifyError("get_agent_reply_shortcut", err));
    invoke<string | null>("get_agent_reply_shortcut_error")
      .then(setRegisterError)
      .catch((err) => notifyError("get_agent_reply_shortcut_error", err));
  }, []);

  useEffect(() => {
    if (!listening) return;
    function onKeyDown(e: KeyboardEvent) {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setListening(false);
        return;
      }
      if (IGNORED_KEYS.has(e.key)) return;
      const next: ShortcutConfig = { meta: e.metaKey, ctrl: e.ctrlKey, alt: e.altKey, shift: e.shiftKey, code: e.code };
      setListening(false);
      setError(null);
      invoke<ShortcutConfig>("set_agent_reply_shortcut", { newCfg: next })
        .then((saved) => {
          setShortcut(saved);
          setRegisterError(null);
        })
        .catch((err) => setError(String(err)));
    }
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [listening]);

  return (
    <div className="rounded-lg border border-base-content/10 bg-base-100 p-4">
      <h3 className="mb-3 text-xs font-semibold uppercase tracking-wide opacity-60">Reply shortcut</h3>
      <button
        className={`btn btn-sm w-full ${listening ? "btn-primary" : ""}`}
        onClick={() => setListening(true)}
        data-testid="agent-reply-shortcut"
      >
        {listening ? "Press a key combo… (Esc to cancel)" : shortcut ? shortcutLabel(shortcut) : "…"}
      </button>
      <p className="mt-1.5 text-xs opacity-60">
        Press it, speak your reply, press it again. The text goes to the agent that has been waiting the shortest time,
        then you land back where you were. End with "press enter" to submit. If Mouthpiece can't confirm the agent's
        tab is the one showing, it pastes without submitting and leaves you there to check and press Return.
      </p>
      {error && <p className="mt-1.5 text-xs text-error">{error}</p>}
      {registerError && (
        <p className="mt-1.5 text-xs text-error">This shortcut couldn't be registered: {registerError}</p>
      )}
    </div>
  );
}

function SoundToggle() {
  const [enabled, setEnabled] = useState<boolean | null>(null);

  useEffect(() => {
    invoke<boolean>("get_agent_sound_enabled")
      .then(setEnabled)
      .catch((err) => notifyError("get_agent_sound_enabled", err));
  }, []);

  function toggle(next: boolean) {
    setEnabled(next);
    invoke("set_agent_sound_enabled", { enabled: next }).catch((err) => {
      notifyError("set_agent_sound_enabled", err);
      setEnabled(!next);
    });
  }

  if (enabled === null) return null;
  return (
    <label className="flex items-center gap-1.5 text-xs">
      <input
        type="checkbox"
        className="checkbox checkbox-xs"
        checked={enabled}
        onChange={(e) => toggle(e.target.checked)}
      />
      Play a soft sound when an agent starts waiting
    </label>
  );
}

function HookSnippet() {
  const [hook, setHook] = useState<HookConfig | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    invoke<HookConfig>("get_agent_hook_config")
      .then(setHook)
      .catch((err) => notifyError("get_agent_hook_config", err));
  }, []);

  function copy() {
    if (!hook) return;
    navigator.clipboard
      .writeText(hook.json)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      })
      .catch((err) => notifyError("Couldn't copy to the clipboard", err));
  }

  return (
    <div className="rounded-lg border border-base-content/10 bg-base-100 p-4">
      <h3 className="mb-3 text-xs font-semibold uppercase tracking-wide opacity-60">Claude Code hook</h3>
      <p className="mb-2 text-xs opacity-60">
        Add this to the <code>hooks</code> in <code>~/.claude/settings.json</code> (merge it with any hooks you already
        have). Notification and Stop tell Mouthpiece an agent is waiting; UserPromptSubmit clears it when you answer in
        the terminal yourself.
      </p>
      {hook ? (
        <>
          <pre
            className="max-h-56 overflow-auto rounded bg-base-300/50 p-2 text-[11px] leading-snug"
            data-testid="agent-hook-json"
          >
            {hook.json}
          </pre>
          <div className="mt-2 flex items-center gap-2">
            <button className="btn btn-sm" onClick={copy}>
              {copied ? "Copied" : "Copy hook config"}
            </button>
            <span className="truncate text-xs opacity-50" title={hook.script_path}>
              Script: {hook.script_path}
            </span>
          </div>
        </>
      ) : (
        <span className="loading loading-spinner loading-xs" />
      )}
    </div>
  );
}

function WaitingList() {
  const [agents, setAgents] = useState<WaitingAgent[]>([]);
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    invoke<WaitingAgent[]>("get_waiting_agents")
      .then(setAgents)
      .catch((err) => notifyError("get_waiting_agents", err));
    const unlisten = listen<WaitingAgent[]>("agents-changed", (e) => setAgents(e.payload));
    const tick = setInterval(() => {
      setNow(Date.now());
      invoke<WaitingAgent[]>("get_waiting_agents").then(setAgents).catch(() => {});
    }, 30_000);
    return () => {
      clearInterval(tick);
      unlisten.then((f) => f());
    };
  }, []);

  function focus(agent: WaitingAgent) {
    invoke("focus_waiting_agent", { sessionId: agent.session_id }).catch((err) =>
      notifyError("Couldn't switch to the agent", err),
    );
  }

  function dismiss(agent: WaitingAgent) {
    invoke("dismiss_waiting_agent", { sessionId: agent.session_id }).catch((err) =>
      notifyError("dismiss_waiting_agent", err),
    );
  }

  return (
    <div className="rounded-lg border border-base-content/10 bg-base-100 p-4">
      <h3 className="mb-3 text-xs font-semibold uppercase tracking-wide opacity-60">Waiting now</h3>
      {agents.length === 0 ? (
        <p className="text-xs opacity-60" data-testid="agents-empty">
          No agent is waiting. Agents appear here when their hook fires and drop off after 30 minutes.
        </p>
      ) : (
        <ul className="flex flex-col gap-1.5" data-testid="agents-list">
          {agents.map((agent) => (
            <li key={agent.session_id} className="flex items-center gap-2 text-xs" data-testid="agent-row">
              <div className="min-w-0 flex-1">
                <div className="truncate font-medium">
                  {agent.folder}
                  <span className="ml-1.5 font-normal opacity-60">
                    {agent.app_name ?? "app unknown"} · {ageLabel(agent.received_ms, now)}
                  </span>
                </div>
                <div className="truncate opacity-60" title={agent.message}>
                  {agent.kind === "done" ? "Finished" : agent.message || "Needs your input"}
                </div>
              </div>
              <button className="btn btn-xs" onClick={() => focus(agent)}>
                Go to window
              </button>
              <button className="btn btn-ghost btn-xs" onClick={() => dismiss(agent)} aria-label={`Dismiss ${agent.folder}`}>
                Dismiss
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export function AgentsSection() {
  return (
    <div className="flex flex-col gap-3" data-testid="agents-section">
      <p className="text-xs opacity-70">
        When a Claude Code agent finishes or asks for permission, Mouthpiece shows a pill in the widget. Press the reply
        shortcut, say your answer, and it is pasted into that agent's window and you are returned to what you were doing.
      </p>
      <HookSnippet />
      <ReplyShortcut />
      <div className="rounded-lg border border-base-content/10 bg-base-100 p-4">
        <h3 className="mb-3 text-xs font-semibold uppercase tracking-wide opacity-60">Notice</h3>
        <SoundToggle />
      </div>
      <WaitingList />
      <div className="rounded-lg border border-base-content/10 bg-base-100 p-4">
        <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide opacity-60">What it can and can't do</h3>
        <ul className="list-disc space-y-1 pl-4 text-xs opacity-70">
          <li>
            Finds the agent's window by looking for its folder name in window titles. VS Code, Cursor and Terminal usually
            show it; iTerm, Orca, Warp and others often don't.
          </li>
          <li>
            When titles don't say, a reply is sent only if the app has a single window. Tabs and split panes inside a window
            can't be selected, so make sure the agent's tab is the one showing.
          </li>
          <li>
            If it can't be sure, the reply is copied to the clipboard instead and the widget says so. It never pastes into
            a different app.
          </li>
          <li>Needs Accessibility permission, the same as pasting.</li>
        </ul>
      </div>
    </div>
  );
}
