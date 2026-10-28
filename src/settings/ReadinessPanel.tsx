import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import { notifyError } from "../ui/toast";
import { FIX_LABEL, getReadiness, type ReadinessCheck, type ReadinessFix } from "./readiness";

// Setup checklist pinned above every Settings page. Hidden entirely once
// everything is ready, so it only ever takes space when it has a job.
// `onNavigate` takes the user to the section that fixes an in-app problem;
// fixes outside the app (Accessibility, the microphone, Ollama) are handled here.
export function ReadinessPanel({ onNavigate }: { onNavigate: (fix: ReadinessFix) => void }) {
  const [checks, setChecks] = useState<ReadinessCheck[] | null>(null);

  useEffect(() => {
    const refresh = () =>
      getReadiness()
        .then(setChecks)
        .catch((err) => notifyError("get_readiness", err));
    refresh();
    // Anything that can change a check: coming back from System Settings,
    // a model finishing, a mic plugging in, a hotkey change.
    window.addEventListener("focus", refresh);
    const unlisteners = ["model-download-complete", "audio-device-status", "llm-pull-complete", "shortcut-changed"].map(
      (event) => listen(event, refresh),
    );
    return () => {
      window.removeEventListener("focus", refresh);
      unlisteners.forEach((u) => u.then((f) => f()));
    };
  }, []);

  const problems = (checks ?? []).filter((c) => c.level !== "ok");
  if (problems.length === 0) return null;

  function fix(kind: ReadinessFix) {
    if (kind === "grant_accessibility") {
      invoke("request_accessibility").catch((err) => notifyError("request_accessibility", err));
    } else if (kind === "allow_microphone") {
      // Shows macOS's own prompt; the list refreshes once it's answered.
      invoke("request_microphone_access")
        .then(() => getReadiness().then(setChecks))
        .catch((err) => notifyError("request_microphone_access", err));
    } else if (kind === "open_microphone_settings") {
      invoke("open_microphone_settings").catch((err) => notifyError("open_microphone_settings", err));
    } else if (kind === "start_ollama") {
      openUrl("https://ollama.com/download").catch((err) =>
        notifyError("Couldn't open the Ollama download page", err),
      );
    } else {
      onNavigate(kind);
    }
  }

  const blocking = problems.some((p) => p.level === "error");
  return (
    <section
      className={`dw-readiness ${blocking ? "is-blocking" : ""}`}
      aria-label="Setup checklist"
      data-testid="readiness-panel"
    >
      <h3>{blocking ? "Finish setup to start dictating" : "Heads up"}</h3>
      <ul>
        {problems.map((check) => (
          <li key={check.id} className={`dw-readiness-item level-${check.level}`} data-testid={`readiness-${check.id}`}>
            <span className="dw-readiness-dot" aria-hidden />
            <div className="dw-readiness-text">
              <strong>{check.title}</strong>
              <span>{check.detail}</span>
            </div>
            {check.fix && (
              <button type="button" className="dw-readiness-fix" onClick={() => fix(check.fix!)}>
                {FIX_LABEL[check.fix]}
              </button>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
