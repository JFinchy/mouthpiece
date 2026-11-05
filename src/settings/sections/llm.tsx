import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { notifyError } from "../../ui/toast";
import { type LlmModelStatus, type ModelFile } from "../shared";


// Ollama cloud models run on Ollama's servers ("gpt-oss:120b-cloud"); the
// backend never lists or uses them (llm::is_cloud_model).
function isCloudModel(model: string | null | undefined) {
  if (!model) return false;
  const name = model.trim().toLowerCase();
  const tag = name.includes(":") ? name.slice(name.lastIndexOf(":") + 1) : "";
  return tag === "cloud" || tag.endsWith("-cloud") || name.endsWith("-cloud");
}

export function LlmSection() {
  const [catalog, setCatalog] = useState<LlmModelStatus[]>([]);
  const [activeModel, setActiveModel] = useState("");
  const [checked, setChecked] = useState(false);
  const [progress, setProgress] = useState<Record<string, { status: string; percent: number | null }>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [ollamaMissing, setOllamaMissing] = useState(false);

  function refresh() {
    Promise.all([invoke<LlmModelStatus[]>("list_llm_catalog"), invoke<string>("get_llm_model")])
      .then(([entries, current]) => {
        setCatalog(entries);
        setActiveModel(current);
        setOllamaMissing(!entries.some((m) => m.downloaded));
      })
      .catch((err) => notifyError("Couldn't load LLM settings", err))
      .finally(() => setChecked(true));
  }

  useEffect(() => {
    refresh();

    const unlistenProgress = listen<{ id: string; status: string; percent: number | null }>(
      "llm-pull-progress",
      (e) => {
        setProgress((p) => ({ ...p, [e.payload.id]: { status: e.payload.status, percent: e.payload.percent } }));
      },
    );
    const unlistenDone = listen<string>("llm-pull-complete", (e) => {
      setProgress((p) => {
        const next = { ...p };
        delete next[e.payload];
        return next;
      });
      refresh();
    });
    const unlistenError = listen<{ id: string; error: string }>("llm-pull-error", (e) => {
      setProgress((p) => {
        const next = { ...p };
        delete next[e.payload.id];
        return next;
      });
      setErrors((er) => ({ ...er, [e.payload.id]: e.payload.error }));
    });

    return () => {
      unlistenProgress.then((f) => f());
      unlistenDone.then((f) => f());
      unlistenError.then((f) => f());
    };
  }, []);

  function selectModel(model: string) {
    setActiveModel(model);
    invoke("set_llm_model", { model }).catch((err) => notifyError("set_llm_model", err));
  }

  function pull(id: string) {
    setErrors((er) => {
      const next = { ...er };
      delete next[id];
      return next;
    });
    setProgress((p) => ({ ...p, [id]: { status: "starting…", percent: null } }));
    invoke("pull_llm_model", { id }).catch((err) => setErrors((er) => ({ ...er, [id]: String(err) })));
  }

  return (
    <div className="mb-4">
      <OnlyWhenNeededToggle />
      <label className="mb-1 block text-xs font-medium opacity-70">LLM refinement</label>
      {!checked ? (
        <span className="loading loading-spinner loading-xs" />
      ) : (
        <>
          {ollamaMissing && (
            <p className="mb-1.5 text-xs opacity-60">
              No local models found. Install{" "}
              <button
                type="button"
                className="link"
                onClick={() => openUrl("https://ollama.com/download").catch((err) => notifyError("Couldn't open ollama.com", err))}
              >
                Ollama
              </button>{" "}
              and download a model below, start LM Studio's, llama.cpp's or Jan's local server, or add a model
              file you already have.
            </p>
          )}
          <ul className="flex flex-col gap-1.5">
            {catalog.map((m) => (
              <li key={m.id} className="flex items-center justify-between rounded-md bg-base-100 px-2.5 py-1.5 text-xs">
                <div className="truncate">
                  <div className="flex items-center gap-1.5 font-medium">
                    <span className="truncate">{m.label}</span>
                    {m.source !== "Ollama" && (
                      <span className="badge badge-ghost badge-xs shrink-0" data-testid="llm-source">
                        {m.source}
                      </span>
                    )}
                  </div>
                  <div className="flex gap-1.5 opacity-50">
                    {m.size_gb > 0 && <span>{m.size_gb.toFixed(1)}GB</span>}
                    {m.last_latency_ms !== null && (
                      <span title="Last observed refinement round-trip time">
                        ~{(m.last_latency_ms / 1000).toFixed(1)}s
                      </span>
                    )}
                  </div>
                </div>
                {m.id === activeModel ? (
                  <span className="badge badge-success badge-sm">Active</span>
                ) : m.id in progress ? (
                  <span className="w-16 truncate text-right opacity-70" title={progress[m.id].status}>
                    {progress[m.id].percent !== null ? `${progress[m.id].percent}%` : progress[m.id].status}
                  </span>
                ) : m.downloaded ? (
                  <button className="btn btn-xs" onClick={() => selectModel(m.id)}>
                    Use
                  </button>
                ) : (
                  <button className="btn btn-xs" onClick={() => pull(m.id)}>
                    Download
                  </button>
                )}
              </li>
            ))}
          </ul>
          {Object.entries(errors).map(([id, err]) => (
            <p key={id} className="mt-1 text-xs text-error">
              {id}: {err}
            </p>
          ))}
          {isCloudModel(activeModel) && (
            <p className="mt-1.5 text-xs text-warning" data-testid="llm-cloud-warning">
              {activeModel} is an Ollama cloud model, so it isn't used. Pick a local model above.
            </p>
          )}
          <p className="mt-1.5 text-xs opacity-50">
            Used by any mode with "Refine with LLM" enabled above. Models from a running LM Studio, llama.cpp or Jan
            server on this Mac are listed too. Ollama cloud models are hidden and never used, so your text stays on
            your Mac.
          </p>
          <ModelFiles onImported={refresh} />
        </>
      )}
    </div>
  );
}

// Refinement takes seconds (the model rewrites the whole message), and on
// a clean transcript it hands back the same text. This skips it unless
// the transcript has filler, a stutter, or no punctuation; Casual mode
// and modes with custom instructions still always refine.
function OnlyWhenNeededToggle() {
  const [enabled, setEnabled] = useState<boolean | null>(null);

  useEffect(() => {
    invoke<boolean>("get_llm_only_when_needed")
      .then(setEnabled)
      .catch((err) => notifyError("get_llm_only_when_needed", err));
  }, []);

  function toggle(next: boolean) {
    setEnabled(next);
    invoke("set_llm_only_when_needed", { enabled: next }).catch((err) => {
      setEnabled(!next);
      notifyError("set_llm_only_when_needed", err);
    });
  }

  if (enabled === null) return null;
  return (
    <div className="mb-3 flex flex-col gap-1">
      <label className="flex items-center gap-1.5 text-xs">
        <input
          type="checkbox"
          className="checkbox checkbox-xs"
          checked={enabled}
          onChange={(e) => toggle(e.target.checked)}
        />
        Only refine when it's needed (faster)
      </label>
      <p className="text-xs opacity-60">
        Clean transcripts paste right away. Refinement still runs when there's filler ("um", "you know"), a
        repeated word, or a long stretch with no punctuation, and always in Casual mode or modes with custom
        instructions.
      </p>
    </div>
  );
}

type ImportState = { stage: string; percent: number | null } | { error: string };

// For Macs where Ollama can't download (a corporate proxy, an allow-listed
// model source) but a model file is already here: adds a .gguf to the local
// Ollama. Files in the usual folders are found automatically; anything else
// (Downloads, a shared drive) is chosen with the file picker.
function ModelFiles({ onImported }: { onImported: () => void }) {
  const [files, setFiles] = useState<ModelFile[] | null>(null);
  const [imports, setImports] = useState<Record<string, ImportState>>({});

  function scan() {
    invoke<ModelFile[]>("find_local_model_files")
      .then((found) =>
        // Keep a picked file in the list after a rescan.
        setFiles((prev) => [
          ...found,
          ...(prev ?? []).filter((p) => p.location === "Chosen file" && !found.some((f) => f.path === p.path)),
        ]),
      )
      .catch((err) => notifyError("Couldn't look for model files", err));
  }

  useEffect(() => {
    scan();
    const unlistenProgress = listen<{ path: string; stage: string; percent: number | null }>(
      "llm-import-progress",
      (e) => setImports((s) => ({ ...s, [e.payload.path]: { stage: e.payload.stage, percent: e.payload.percent } })),
    );
    const unlistenDone = listen<{ path: string; model: string }>("llm-import-complete", (e) => {
      setImports((s) => {
        const next = { ...s };
        delete next[e.payload.path];
        return next;
      });
      setFiles((prev) =>
        (prev ?? []).map((f) => (f.path === e.payload.path ? { ...f, in_ollama: true, ollama_name: e.payload.model } : f)),
      );
      onImported();
    });
    const unlistenError = listen<{ path: string; error: string }>("llm-import-error", (e) =>
      setImports((s) => ({ ...s, [e.payload.path]: { error: e.payload.error } })),
    );
    return () => {
      unlistenProgress.then((f) => f());
      unlistenDone.then((f) => f());
      unlistenError.then((f) => f());
    };
  }, []);

  function add(file: ModelFile) {
    setImports((s) => ({ ...s, [file.path]: { stage: "Starting…", percent: null } }));
    invoke("import_model_file", { path: file.path }).catch((err) =>
      setImports((s) => ({ ...s, [file.path]: { error: String(err) } })),
    );
  }

  function choose() {
    invoke<ModelFile | null>("choose_model_file")
      .then((file) => {
        if (!file) return;
        setFiles((prev) => [file, ...(prev ?? []).filter((f) => f.path !== file.path)]);
        if (!file.in_ollama) add(file);
      })
      .catch((err) => notifyError("Couldn't use that file", err));
  }

  return (
    <div className="mt-4" data-testid="model-files">
      <div className="mb-1 flex items-center justify-between">
        <label className="text-xs font-medium opacity-70">Model files on this Mac</label>
        <button type="button" className="btn btn-xs" onClick={choose}>
          Choose a .gguf file…
        </button>
      </div>
      <p className="mb-1.5 text-xs opacity-60">
        Already have a model file? Add it to Ollama without downloading anything. Ollama has to be running; the
        file is copied into it, so it needs that much free disk space.
      </p>
      {files === null ? (
        <span className="loading loading-spinner loading-xs" />
      ) : files.length === 0 ? (
        <p className="text-xs opacity-50">
          None found in LM Studio's, Jan's or the Hugging Face folders, or ~/models.
        </p>
      ) : (
        <ul className="flex flex-col gap-1.5">
          {files.map((f) => {
            const state = imports[f.path];
            return (
              <li key={f.path} className="rounded-md bg-base-100 px-2.5 py-1.5 text-xs">
                <div className="flex items-center justify-between gap-2">
                  <div className="min-w-0">
                    <div className="truncate font-medium" title={f.path}>
                      {f.file_name}
                    </div>
                    <div className="flex gap-1.5 opacity-50">
                      <span>{f.size_gb.toFixed(1)}GB</span>
                      <span>{f.location}</span>
                    </div>
                  </div>
                  {f.in_ollama ? (
                    <span className="shrink-0 opacity-70" title={`In Ollama as ${f.ollama_name}`}>
                      In Ollama ✓
                    </span>
                  ) : state && "stage" in state ? (
                    <span className="w-28 shrink-0 truncate text-right opacity-70" title={state.stage}>
                      {state.stage}
                      {state.percent !== null ? ` ${state.percent}%` : "…"}
                    </span>
                  ) : (
                    <button type="button" className="btn btn-xs shrink-0" onClick={() => add(f)}>
                      Add to Ollama
                    </button>
                  )}
                </div>
                {state && "error" in state && <p className="mt-1 text-error">{state.error}</p>}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
