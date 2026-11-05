import { useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { notifyError } from "../../ui/toast";
import { BEHAVIORS, BEHAVIOR_LABEL, type Behavior, type LlmModelStatus, type ModelStatus } from "../shared";

export const GLOBAL_MODEL = "__global__";

export type AppRef = { bundle_id: string; app_name: string };

// Mirrors the Rust `LlmRefinement` enum's serde shape: unit variants
// serialize as bare strings, the tuple variant as `{ model: "<id>" }`.
export type LlmRefinement = "off" | "global" | { model: string };

export type ModeDefinition = {
  name: string;
  behavior: Behavior;
  apps: AppRef[];
  stt_model: string | null;
  llm_refinement: LlmRefinement;
  custom_instructions: string | null;
  is_default: boolean;
};

export type FrontmostApp = { bundle_id: string; name: string; icon_data_uri: string | null };

export type RunningApp = { bundle_id: string; name: string; icon_data_uri: string | null; is_running: boolean };

export function llmSelectValue(r: LlmRefinement): string {
  return typeof r === "string" ? r : r.model;
}

export function llmFromSelectValue(v: string): LlmRefinement {
  return v === "off" || v === "global" ? v : { model: v };
}

export function AppIcon({ src, name }: { src: string | null; name: string }) {
  if (src) {
    return <img src={src} alt="" className="h-4 w-4 shrink-0 rounded-sm" />;
  }
  // Fallback for apps whose icon couldn't be resolved: first letter avatar.
  return (
    <span className="flex h-4 w-4 shrink-0 items-center justify-center rounded-sm bg-base-300 text-[9px] font-semibold opacity-60">
      {name.charAt(0).toUpperCase()}
    </span>
  );
}

export function InfoPopover({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <span className="relative inline-block">
      <button
        type="button"
        className="btn btn-ghost btn-circle btn-xs opacity-60 hover:opacity-100"
        aria-label="More info"
        onClick={() => setOpen((v) => !v)}
        onBlur={() => setOpen(false)}
      >
        ⓘ
      </button>
      {open && (
        <div className="absolute right-0 z-20 mt-1 w-64 rounded-md border border-base-content/10 bg-base-100 p-2.5 text-[11px] leading-snug opacity-100 shadow-lg">
          {children}
        </div>
      )}
    </span>
  );
}

export function AppModesSection() {
  const [modes, setModes] = useState<ModeDefinition[]>([]);
  const [loading, setLoading] = useState(true);
  const [lastApp, setLastApp] = useState<FrontmostApp | null>(null);
  const [runningApps, setRunningApps] = useState<RunningApp[]>([]);
  const [availableModels, setAvailableModels] = useState<ModelStatus[]>([]);
  const [llmCatalog, setLlmCatalog] = useState<LlmModelStatus[]>([]);
  const [pickerOpenFor, setPickerOpenFor] = useState<number | null>(null);
  const [llmProgress, setLlmProgress] = useState<Record<string, { status: string; percent: number | null }>>({});
  const [llmErrors, setLlmErrors] = useState<Record<string, string>>({});

  function refresh() {
    invoke<ModeDefinition[]>("get_modes")
      .then(setModes)
      .catch((err) => notifyError("get_modes", err))
      .finally(() => setLoading(false));
  }

  function loadRunningApps() {
    invoke<RunningApp[]>("list_running_apps")
      .then(setRunningApps)
      .catch((err) => notifyError("list_running_apps", err));
  }

  function loadLlmCatalog() {
    invoke<LlmModelStatus[]>("list_llm_catalog")
      .then(setLlmCatalog)
      .catch((err) => notifyError("list_llm_catalog", err));
  }

  function pullLlmModel(id: string) {
    setLlmErrors((er) => {
      const next = { ...er };
      delete next[id];
      return next;
    });
    setLlmProgress((p) => ({ ...p, [id]: { status: "starting…", percent: null } }));
    invoke("pull_llm_model", { id }).catch((err) => setLlmErrors((er) => ({ ...er, [id]: String(err) })));
  }

  useEffect(() => {
    refresh();
    loadRunningApps();
    loadLlmCatalog();
    invoke<FrontmostApp | null>("get_last_frontmost_app")
      .then(setLastApp)
      .catch((err) => notifyError("get_last_frontmost_app", err));
    invoke<ModelStatus[]>("list_models")
      .then(setAvailableModels)
      .catch((err) => notifyError("list_models", err));

    const unlistenProgress = listen<{ id: string; status: string; percent: number | null }>(
      "llm-pull-progress",
      (e) => {
        setLlmProgress((p) => ({ ...p, [e.payload.id]: { status: e.payload.status, percent: e.payload.percent } }));
      },
    );
    const unlistenDone = listen<string>("llm-pull-complete", (e) => {
      setLlmProgress((p) => {
        const next = { ...p };
        delete next[e.payload];
        return next;
      });
      loadLlmCatalog();
    });
    const unlistenError = listen<{ id: string; error: string }>("llm-pull-error", (e) => {
      setLlmProgress((p) => {
        const next = { ...p };
        delete next[e.payload.id];
        return next;
      });
      setLlmErrors((er) => ({ ...er, [e.payload.id]: e.payload.error }));
    });

    return () => {
      unlistenProgress.then((f) => f());
      unlistenDone.then((f) => f());
      unlistenError.then((f) => f());
    };
  }, []);

  // Full-list replace, mirroring `SnippetsSection` — the whole list lives
  // client-side (renaming a mode, adding/removing an app, adding/deleting
  // a mode are all just array edits) and gets saved back in one shot
  // rather than the backend exposing per-field mutation commands.
  function save(next: ModeDefinition[]) {
    setModes(next);
    invoke("set_modes", { modes: next }).catch((err) => notifyError("set_modes", err));
  }

  function updateMode(index: number, patch: Partial<ModeDefinition>) {
    const next = modes.slice();
    next[index] = { ...next[index], ...patch };
    save(next);
  }

  function addMode() {
    save([
      ...modes,
      {
        name: "New mode",
        behavior: "plain",
        apps: [],
        stt_model: null,
        llm_refinement: "off",
        custom_instructions: null,
        is_default: false,
      },
    ]);
  }

  function removeMode(index: number) {
    save(modes.filter((_, i) => i !== index));
  }

  function addAppToMode(index: number, bundleId: string, appName: string) {
    const next = modes.slice();
    next[index] = { ...next[index], apps: [...next[index].apps, { bundle_id: bundleId, app_name: appName }] };
    save(next);
    setPickerOpenFor(null);
  }

  function removeAppFromMode(index: number, bundleId: string) {
    const next = modes.slice();
    next[index] = { ...next[index], apps: next[index].apps.filter((a) => a.bundle_id !== bundleId) };
    save(next);
  }

  const assignedBundleIds = new Set(modes.flatMap((m) => m.apps.map((a) => a.bundle_id)));
  const lastAppAlreadyAssigned = lastApp && assignedBundleIds.has(lastApp.bundle_id);
  const appIcon = (bundleId: string) => runningApps.find((a) => a.bundle_id === bundleId)?.icon_data_uri ?? null;

  if (loading) {
    return <span className="loading loading-spinner loading-xs" />;
  }

  return (
    <>
      <div className="flex flex-col gap-3">
        {modes.map((m, index) => {
          const pickableApps = runningApps.filter((a) => !assignedBundleIds.has(a.bundle_id));
          return (
            <div key={index} className="rounded-lg border border-base-content/10 bg-base-100 p-4">
              <div className="mb-2 flex items-center justify-between gap-2">
                <input
                  className="input input-xs flex-1 font-medium"
                  value={m.name}
                  onChange={(e) => updateMode(index, { name: e.target.value })}
                  aria-label="Mode name"
                />
                {!m.is_default && (
                  <button
                    className="btn btn-ghost btn-xs"
                    onClick={() => removeMode(index)}
                    aria-label={`Delete ${m.name}`}
                  >
                    ✕
                  </button>
                )}
              </div>

              <div className="mb-2 flex flex-wrap items-center gap-1.5">
                {m.apps.map((a) => (
                  <span
                    key={a.bundle_id}
                    className="flex items-center gap-1 rounded-full bg-base-200 py-0.5 pl-1 pr-1.5 text-[11px]"
                  >
                    <AppIcon src={appIcon(a.bundle_id)} name={a.app_name} />
                    {a.app_name}
                    <button
                      className="opacity-50 hover:opacity-100"
                      onClick={() => removeAppFromMode(index, a.bundle_id)}
                      aria-label={`Remove ${a.app_name} from ${m.name}`}
                    >
                      ✕
                    </button>
                  </span>
                ))}
                <div className="relative">
                  <button
                    className="btn btn-ghost btn-xs"
                    onClick={() => {
                      setPickerOpenFor(pickerOpenFor === index ? null : index);
                      loadRunningApps();
                    }}
                  >
                    + Add app
                  </button>
                  {pickerOpenFor === index && (
                    <ul className="absolute z-10 mt-1 max-h-52 w-48 overflow-y-auto rounded-md bg-base-100 p-1 shadow-lg">
                      {pickableApps.length === 0 && <li className="px-2 py-1 text-xs opacity-50">Loading…</li>}
                      {pickableApps.map((a, i) => {
                        const prevRunning = i > 0 ? pickableApps[i - 1].is_running : true;
                        return (
                          <li key={a.bundle_id}>
                            {prevRunning && !a.is_running && (
                              <div className="mt-1 border-t border-base-content/10 pt-1 text-[10px] uppercase opacity-40">
                                Other apps
                              </div>
                            )}
                            <button
                              className="flex w-full items-center gap-1.5 rounded px-2 py-1 text-left text-xs hover:bg-base-200"
                              onClick={() => addAppToMode(index, a.bundle_id, a.name)}
                            >
                              <AppIcon src={a.icon_data_uri} name={a.name} />
                              <span className="truncate">{a.name}</span>
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  )}
                </div>
              </div>

              <div className="flex flex-wrap items-center gap-1.5">
                <select
                  className="select select-xs"
                  value={m.behavior}
                  onChange={(e) => updateMode(index, { behavior: e.target.value as Behavior })}
                >
                  {BEHAVIORS.map((b) => (
                    <option key={b} value={b}>
                      {BEHAVIOR_LABEL[b]}
                    </option>
                  ))}
                </select>
                <select
                  className="select select-xs"
                  value={m.stt_model ?? GLOBAL_MODEL}
                  onChange={(e) =>
                    updateMode(index, { stt_model: e.target.value === GLOBAL_MODEL ? null : e.target.value })
                  }
                >
                  <option value={GLOBAL_MODEL}>Global model</option>
                  {availableModels.map((mo) => (
                    <option key={mo.id} value={mo.id}>
                      {mo.label}
                    </option>
                  ))}
                </select>
                <select
                  className="select select-xs"
                  value={llmSelectValue(m.llm_refinement)}
                  onChange={(e) => updateMode(index, { llm_refinement: llmFromSelectValue(e.target.value) })}
                >
                  <option value="off">No LLM refinement</option>
                  <option value="global">Refine with global LLM</option>
                  {llmCatalog.map((l) => (
                    <option key={l.id} value={l.id}>
                      {l.downloaded ? "✓ " : "⬇ "}
                      {l.label}
                      {l.source !== "Ollama" ? ` · ${l.source}` : ""}
                      {!l.downloaded && l.size_gb > 0 ? ` — ${l.size_gb.toFixed(1)}GB, not installed` : ""}
                    </option>
                  ))}
                </select>
                <InfoPopover>
                  <p className="mb-1 font-semibold">Installing a model</p>
                  <p>
                    Models run fully offline via Ollama, or a local LM Studio, llama.cpp or Jan server. "✓" means it's already downloaded and ready to use for free;
                    "⬇" means selecting it triggers a one-time download (the size shown) that's kept permanently on
                    disk. No ongoing cost either way — no data ever leaves this machine, and there's no per-use fee.
                  </p>
                </InfoPopover>
              </div>

              {(() => {
                const selectedModelId = typeof m.llm_refinement === "object" ? m.llm_refinement.model : null;
                const selected = selectedModelId ? llmCatalog.find((l) => l.id === selectedModelId) : undefined;
                if (!selectedModelId || !selected || selected.downloaded) return null;
                const progress = llmProgress[selectedModelId];
                const error = llmErrors[selectedModelId];
                return (
                  <div className="mt-1.5 flex items-center gap-1.5 text-[11px] text-warning">
                    <span>
                      {selected.label} isn't installed yet{selected.size_gb > 0 ? ` (${selected.size_gb.toFixed(1)}GB)` : ""}.
                    </span>
                    {progress ? (
                      <span className="opacity-70" title={progress.status}>
                        {progress.percent !== null ? `${progress.percent}%` : progress.status}
                      </span>
                    ) : (
                      <button className="btn btn-xs" onClick={() => pullLlmModel(selectedModelId)}>
                        Download
                      </button>
                    )}
                    {error && <span className="text-error">{error}</span>}
                  </div>
                );
              })()}

              {llmSelectValue(m.llm_refinement) !== "off" && (
                <textarea
                  className="textarea textarea-xs mt-2 w-full font-sans"
                  rows={2}
                  aria-label={`Extra LLM instructions for ${m.name}`}
                  placeholder={
                    'Extra instructions for the LLM, e.g. "Always sign off with \'thanks, Jake\'."\n\n' +
                    'Example — said: "hey can we push the meeting to thursday" -> output: "Hi, could we push ' +
                    'our meeting to Thursday?\\n\\nThanks,\\nJake"'
                  }
                  value={m.custom_instructions ?? ""}
                  onChange={(e) =>
                    updateMode(index, { custom_instructions: e.target.value === "" ? null : e.target.value })
                  }
                />
              )}
            </div>
          );
        })}
        <button className="btn btn-xs w-full justify-start" onClick={addMode}>
          + Add mode
        </button>
      </div>

      {lastApp && !lastAppAlreadyAssigned && (
        <div className="mt-1.5 flex items-center gap-1.5 text-xs">
          <AppIcon src={lastApp.icon_data_uri} name={lastApp.name} />
          <span className="truncate opacity-70">Add {lastApp.name} to:</span>
          <select
            className="select select-xs flex-1"
            value=""
            onChange={(e) => {
              const index = modes.findIndex((m) => m.name === e.target.value);
              if (index >= 0) addAppToMode(index, lastApp.bundle_id, lastApp.name);
            }}
          >
            <option value="" disabled>
              Choose a mode…
            </option>
            {modes.map((m) => (
              <option key={m.name} value={m.name}>
                {m.name}
              </option>
            ))}
          </select>
        </div>
      )}
    </>
  );
}
