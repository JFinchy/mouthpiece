import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { notifyError } from "../../ui/toast";
import { BEHAVIORS, BEHAVIOR_LABEL, type Behavior } from "../shared";
import { HistoryPrivacy, LockedHistory, useHistoryLock } from "./privacy";

export type HistoryEntry = {
  timestamp_ms: number;
  text: string;
  app_name: string | null;
  mode: string | null;
  summary: string | null;
  /// Text before the LLM rewrote it; present only on refined entries.
  original_text?: string | null;
};

export const RETENTION_OPTIONS = [7, 30, 90, 365];

export function formatTimestamp(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export type ReprocessState = {
  mode: Behavior;
  useLlm: boolean;
  loading: boolean;
  result: string | null;
  error: string | null;
};

export type DiffPair = {
  from: string;
  to: string;
};

export type DiffToken = {
  kind: "eq" | "del" | "ins";
  text: string;
  /** Set when this token belongs to an adjacent del -> ins replacement span. */
  pair?: DiffPair;
};

/** Strip leading/trailing punctuation & whitespace so a diff word is clean for Vocabulary. */
export function cleanVocabWord(raw: string): string {
  return raw
    .trim()
    .replace(/^[\s.,!?;:"'()[\]{}«»“”‘’—–-]+|[\s.,!?;:"'()[\]{}«»“”‘’—–-]+$/g, "")
    .trim();
}

const MAX_LCS_CELLS = 40_000;

function finalizeDiffTokens(raw: DiffToken[]): DiffToken[] {
  const merged: DiffToken[] = [];
  let idx = 0;

  while (idx < raw.length) {
    if (raw[idx].kind === "eq") {
      const parts: string[] = [];
      while (idx < raw.length && raw[idx].kind === "eq") {
        parts.push(raw[idx].text);
        idx++;
      }
      merged.push({ kind: "eq", text: parts.join(" ") });
      continue;
    }

    // Group each contiguous changed span with deletions before insertions so
    // replacement pairs (e.g. "post gress" -> "Postgres") stay adjacent and linked.
    const dels: string[] = [];
    const ins: string[] = [];
    while (idx < raw.length && raw[idx].kind !== "eq") {
      if (raw[idx].kind === "del") dels.push(raw[idx].text);
      else ins.push(raw[idx].text);
      idx++;
    }

    const delText = dels.join(" ");
    const insText = ins.join(" ");
    if (dels.length > 0 && ins.length > 0) {
      const pair: DiffPair = { from: delText, to: insText };
      merged.push({ kind: "del", text: delText, pair });
      merged.push({ kind: "ins", text: insText, pair });
    } else if (dels.length > 0) {
      merged.push({ kind: "del", text: delText });
    } else if (ins.length > 0) {
      merged.push({ kind: "ins", text: insText });
    }
  }

  return merged;
}

export function computeWordDiff(before: string, after: string): DiffToken[] {
  const a = before.trim().split(/\s+/).filter(Boolean);
  const b = after.trim().split(/\s+/).filter(Boolean);
  if (a.length === 0 && b.length === 0) return [];
  if (a.length === 0) return [{ kind: "ins", text: b.join(" ") }];
  if (b.length === 0) return [{ kind: "del", text: a.join(" ") }];

  const raw: DiffToken[] = [];

  if (a.length * b.length > MAX_LCS_CELLS) {
    let start = 0;
    while (start < a.length && start < b.length && a[start] === b[start]) start++;
    let endA = a.length - 1;
    let endB = b.length - 1;
    while (endA >= start && endB >= start && a[endA] === b[endB]) {
      endA--;
      endB--;
    }
    if (start > 0) raw.push({ kind: "eq", text: a.slice(0, start).join(" ") });
    if (endA >= start) raw.push({ kind: "del", text: a.slice(start, endA + 1).join(" ") });
    if (endB >= start) raw.push({ kind: "ins", text: b.slice(start, endB + 1).join(" ") });
    if (endA + 1 < a.length) raw.push({ kind: "eq", text: a.slice(endA + 1).join(" ") });
    return finalizeDiffTokens(raw);
  }

  const n = a.length;
  const m = b.length;
  const stride = m + 1;
  const dp = new Uint16Array((n + 1) * stride);

  for (let i = n - 1; i >= 0; i--) {
    const row = i * stride;
    const nextRow = (i + 1) * stride;
    for (let j = m - 1; j >= 0; j--) {
      dp[row + j] =
        a[i] === b[j]
          ? dp[nextRow + j + 1] + 1
          : Math.max(dp[nextRow + j], dp[row + j + 1]);
    }
  }

  let i = 0;
  let j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && a[i] === b[j]) {
      raw.push({ kind: "eq", text: a[i] });
      i++;
      j++;
    } else if (i < n && (j === m || dp[(i + 1) * stride + j] >= dp[i * stride + j + 1])) {
      raw.push({ kind: "del", text: a[i] });
      i++;
    } else {
      raw.push({ kind: "ins", text: b[j] });
      j++;
    }
  }

  return finalizeDiffTokens(raw);
}

function IconEdit() {
  return (
    <svg
      width="13"
      height="13"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M11.3 2.5a1.5 1.5 0 0 1 2.2 2.2l-7.9 7.9-3 .8.8-3 7.9-7.9Z" />
      <path d="m9.8 4 2.2 2.2" />
    </svg>
  );
}

function IconReprocess() {
  return (
    <svg
      width="13"
      height="13"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M13.5 8a5.5 5.5 0 1 1-1.4-3.7" />
      <path d="M13.5 2.7v2.8h-2.8" />
    </svg>
  );
}

function IconCopy() {
  return (
    <svg
      width="13"
      height="13"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
      <path d="M3.5 10.5H3A1.5 1.5 0 0 1 1.5 9V3A1.5 1.5 0 0 1 3 1.5h6A1.5 1.5 0 0 1 10.5 3v.5" />
    </svg>
  );
}

function IconCheck() {
  return (
    <svg
      width="13"
      height="13"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="m3.5 8.5 3 3 6-6.5" />
    </svg>
  );
}

function IconTrash() {
  return (
    <svg
      width="13"
      height="13"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M2.5 4.2h11" />
      <path d="M5.8 4.2V3a1 1 0 0 1 1-1h2.4a1 1 0 0 1 1 1v1.2" />
      <path d="M4 4.2 4.6 13a1.2 1.2 0 0 0 1.2 1h4.4a1.2 1.2 0 0 0 1.2-1l.6-8.8" />
    </svg>
  );
}

type DiffSelection = {
  tokenIdx: number;
  word: string;
  pair: DiffPair | null;
};

type TeachFeedback = {
  vocabAdded?: string;
  ruleSaved?: string;
  busyVocab?: boolean;
  busyRule?: boolean;
};

function HistoryContent() {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [retentionDays, setRetentionDays] = useState<number>(30);
  const [journalEnabled, setJournalEnabledState] = useState(false);
  const [loading, setLoading] = useState(true);
  const [copiedAt, setCopiedAt] = useState<number | null>(null);
  const [query, setQuery] = useState("");
  const [reprocessing, setReprocessing] = useState<Record<number, ReprocessState>>({});
  const [showOriginal, setShowOriginal] = useState<Set<number>>(new Set());
  const [editing, setEditing] = useState<Record<number, string>>({});
  const [diffSelection, setDiffSelection] = useState<Record<number, DiffSelection>>({});
  const [teachFeedback, setTeachFeedback] = useState<Record<number, TeachFeedback>>({});

  const searchSeqRef = useRef(0);
  const initialQueryMountRef = useRef(true);
  const copyTimerRef = useRef<number | null>(null);
  const vocabQueueRef = useRef<Promise<void>>(Promise.resolve());
  const ruleQueueRef = useRef<Promise<void>>(Promise.resolve());

  useEffect(() => {
    return () => {
      if (copyTimerRef.current !== null) window.clearTimeout(copyTimerRef.current);
    };
  }, []);

  // Empty query -> the normal recent-first list; non-empty -> a full-text
  // scan of the entire retained history, not just what's currently loaded.
  function fetchEntries(q: string): Promise<HistoryEntry[]> {
    const trimmed = q.trim();
    return trimmed
      ? invoke<HistoryEntry[]>("search_history_entries", { query: trimmed })
      : invoke<HistoryEntry[]>("list_history_entries");
  }

  function refresh() {
    const seq = ++searchSeqRef.current;
    fetchEntries(query)
      .then((list) => {
        if (seq !== searchSeqRef.current) return;
        setEntries(list);
      })
      .catch((err) => {
        if (seq !== searchSeqRef.current) return;
        notifyError("Couldn't load history", err);
      });
  }

  useEffect(() => {
    let disposed = false;
    const seq = ++searchSeqRef.current;
    Promise.all([
      fetchEntries(""),
      invoke<number>("get_history_retention_days"),
      invoke<boolean>("get_journal_enabled"),
    ])
      .then(([historyEntries, days, journal]) => {
        if (disposed) return;
        setRetentionDays(days);
        setJournalEnabledState(journal);
        if (seq === searchSeqRef.current) {
          setEntries(historyEntries);
        }
      })
      .catch((err) => {
        if (disposed || seq !== searchSeqRef.current) return;
        notifyError("Couldn't load history", err);
      })
      .finally(() => {
        if (!disposed) setLoading(false);
      });
    return () => {
      disposed = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Debounced re-fetch on every search keystroke. Re-subscribing
  // "history-updated" here (rather than once at mount) is deliberate — it
  // keeps the handler closed over the *current* query, so a new transcript
  // landing mid-search re-filters against that search instead of silently
  // reverting to the unfiltered list.
  useEffect(() => {
    let handle: ReturnType<typeof setTimeout> | null = null;
    if (initialQueryMountRef.current) {
      initialQueryMountRef.current = false;
    } else {
      handle = setTimeout(refresh, 150);
    }
    const unlisten = listen("history-updated", refresh);
    return () => {
      if (handle !== null) clearTimeout(handle);
      unlisten.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  function changeRetention(days: number) {
    const previous = retentionDays;
    setRetentionDays(days);
    invoke("set_history_retention_days", { days })
      .then(refresh) // purging may have removed entries outside the new window
      .catch((err) => {
        notifyError("set_history_retention_days", err);
        setRetentionDays(previous);
      });
  }

  function toggleJournal(enabled: boolean) {
    setJournalEnabledState(enabled);
    invoke("set_journal_enabled", { enabled }).catch((err) => {
      notifyError("set_journal_enabled", err);
      setJournalEnabledState(!enabled);
    });
  }

  function clear() {
    setEntries([]);
    setEditing({});
    setReprocessing({});
    setShowOriginal(new Set());
    setDiffSelection({});
    setTeachFeedback({});
    invoke("clear_history").catch((err) => {
      notifyError("clear_history", err);
      refresh();
    });
  }

  function copyEntry(entry: HistoryEntry) {
    navigator.clipboard
      .writeText(entry.text)
      .then(() => {
        setCopiedAt(entry.timestamp_ms);
        if (copyTimerRef.current !== null) window.clearTimeout(copyTimerRef.current);
        copyTimerRef.current = window.setTimeout(() => {
          copyTimerRef.current = null;
          setCopiedAt((current) => (current === entry.timestamp_ms ? null : current));
        }, 1500);
      })
      .catch((err) => notifyError("Couldn't copy to the clipboard", err));
  }

  function deleteEntry(timestampMs: number) {
    setEntries((current) => current.filter((e) => e.timestamp_ms !== timestampMs));
    setReprocessing((current) => {
      const { [timestampMs]: _removed, ...rest } = current;
      return rest;
    });
    setEditing((current) => {
      if (!(timestampMs in current)) return current;
      const { [timestampMs]: _removed, ...rest } = current;
      return rest;
    });
    setShowOriginal((current) => {
      if (!current.has(timestampMs)) return current;
      const next = new Set(current);
      next.delete(timestampMs);
      return next;
    });
    clearDiffSelection(timestampMs);
    invoke("delete_history_entry", { timestampMs }).catch((err) => {
      notifyError("delete_history_entry", err);
      refresh();
    });
  }

  function toggleReprocess(timestampMs: number) {
    setReprocessing((current) => {
      if (current[timestampMs]) {
        const { [timestampMs]: _removed, ...rest } = current;
        return rest;
      }
      return { ...current, [timestampMs]: { mode: "plain", useLlm: false, loading: false, result: null, error: null } };
    });
  }

  function patchReprocess(timestampMs: number, patch: Partial<ReprocessState>) {
    setReprocessing((current) => {
      const existing = current[timestampMs];
      if (!existing) return current;
      return { ...current, [timestampMs]: { ...existing, ...patch } };
    });
  }

  function runReprocess(entry: HistoryEntry) {
    const state = reprocessing[entry.timestamp_ms];
    if (!state) return;
    patchReprocess(entry.timestamp_ms, { loading: true, error: null, result: null });
    invoke<string>("reprocess_history_text", { text: entry.text, mode: state.mode, useLlmRefinement: state.useLlm })
      .then((result) => patchReprocess(entry.timestamp_ms, { loading: false, result }))
      .catch((err) => patchReprocess(entry.timestamp_ms, { loading: false, error: String(err) }));
  }

  function replaceWithReprocessed(entry: HistoryEntry) {
    const state = reprocessing[entry.timestamp_ms];
    if (!state?.result) return;
    invoke("update_history_entry_text", { timestampMs: entry.timestamp_ms, text: state.result, learn: false })
      .then(() => {
        clearDiffSelection(entry.timestamp_ms);
        setReprocessing((current) => {
          const { [entry.timestamp_ms]: _removed, ...rest } = current;
          return rest;
        });
        refresh();
      })
      .catch((err) => notifyError("update_history_entry_text", err));
  }

  function clearDiffSelection(timestampMs: number) {
    setDiffSelection((current) => {
      if (!(timestampMs in current)) return current;
      const { [timestampMs]: _removed, ...rest } = current;
      return rest;
    });
    setTeachFeedback((current) => {
      if (!(timestampMs in current)) return current;
      const { [timestampMs]: _removed, ...rest } = current;
      return rest;
    });
  }

  function toggleOriginal(timestampMs: number) {
    const isClosing = showOriginal.has(timestampMs);
    if (isClosing) {
      clearDiffSelection(timestampMs);
    }
    setShowOriginal((current) => {
      const next = new Set(current);
      if (next.has(timestampMs)) {
        next.delete(timestampMs);
      } else {
        next.add(timestampMs);
      }
      return next;
    });
  }

  function selectDiffToken(timestampMs: number, tokenIdx: number, tok: DiffToken) {
    const word = cleanVocabWord(tok.text);
    if (!word) return;
    setDiffSelection((current) => {
      if (current[timestampMs]?.tokenIdx === tokenIdx) {
        const { [timestampMs]: _removed, ...rest } = current;
        return rest;
      }
      let pair: DiffPair | null = null;
      if (tok.pair) {
        const from = cleanVocabWord(tok.pair.from);
        const to = cleanVocabWord(tok.pair.to);
        const fromWords = from ? from.split(/\s+/).length : 0;
        const toWords = to ? to.split(/\s+/).length : 0;
        const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, " ").trim();
        if (from && to && fromWords <= 3 && toWords <= 3 && norm(from) !== norm(to)) {
          pair = { from, to };
        }
      }
      return { ...current, [timestampMs]: { tokenIdx, word, pair } };
    });
    setTeachFeedback((current) => {
      if (!(timestampMs in current)) return current;
      const { [timestampMs]: _removed, ...rest } = current;
      return rest;
    });
  }

  function teachVocabulary(timestampMs: number, word: string) {
    setTeachFeedback((current) => ({
      ...current,
      [timestampMs]: { ...current[timestampMs], busyVocab: true },
    }));
    const task = vocabQueueRef.current.then(async () => {
      try {
        const existing = await invoke<string[]>("get_vocabulary");
        const needle = word.toLowerCase();
        if (!existing.some((t) => t.toLowerCase() === needle)) {
          await invoke("set_vocabulary", { terms: [...existing, word] });
        }
        setTeachFeedback((current) => ({
          ...current,
          [timestampMs]: { ...current[timestampMs], busyVocab: false, vocabAdded: word },
        }));
      } catch (err) {
        setTeachFeedback((current) => ({
          ...current,
          [timestampMs]: { ...current[timestampMs], busyVocab: false },
        }));
        notifyError("Couldn't save vocabulary", err);
      }
    });
    vocabQueueRef.current = task.catch(() => {});
  }

  function teachReplacement(timestampMs: number, from: string, to: string) {
    setTeachFeedback((current) => ({
      ...current,
      [timestampMs]: { ...current[timestampMs], busyRule: true },
    }));
    const task = ruleQueueRef.current.then(async () => {
      try {
        const existing = await invoke<{ from: string; to: string }[]>("get_replacements");
        const fromLower = from.toLowerCase();
        const filtered = existing.filter((r) => r.from.toLowerCase() !== fromLower);
        const alreadyExact = existing.some(
          (r) => r.from.toLowerCase() === fromLower && r.to === to,
        );
        if (!alreadyExact) {
          await invoke("set_replacements", { replacements: [...filtered, { from, to }] });
        }
        setTeachFeedback((current) => ({
          ...current,
          [timestampMs]: { ...current[timestampMs], busyRule: false, ruleSaved: `${from} → ${to}` },
        }));
      } catch (err) {
        setTeachFeedback((current) => ({
          ...current,
          [timestampMs]: { ...current[timestampMs], busyRule: false },
        }));
        notifyError("Couldn't save replacement rule", err);
      }
    });
    ruleQueueRef.current = task.catch(() => {});
  }

  function copyOriginal(entry: HistoryEntry) {
    navigator.clipboard
      .writeText(entry.original_text ?? "")
      .catch((err) => notifyError("Couldn't copy to the clipboard", err));
  }

  function useOriginal(entry: HistoryEntry) {
    clearDiffSelection(entry.timestamp_ms);
    invoke("use_original_history_text", { timestampMs: entry.timestamp_ms })
      .then(refresh)
      .catch((err) => notifyError("use_original_history_text", err));
  }

  function startEdit(entry: HistoryEntry) {
    setEditing((current) => ({ ...current, [entry.timestamp_ms]: entry.text }));
  }

  function stopEdit(timestampMs: number) {
    setEditing((current) => {
      const { [timestampMs]: _removed, ...rest } = current;
      return rest;
    });
  }

  function saveEdit(entry: HistoryEntry) {
    const text = editing[entry.timestamp_ms]?.trim();
    if (!text || text === entry.text) {
      stopEdit(entry.timestamp_ms);
      return;
    }
    invoke("update_history_entry_text", { timestampMs: entry.timestamp_ms, text, learn: true })
      .then(() => {
        clearDiffSelection(entry.timestamp_ms);
        stopEdit(entry.timestamp_ms);
        refresh();
      })
      .catch((err) => notifyError("update_history_entry_text", err));
  }

  return (
    <div className="mb-4">
      <div className="mb-1 flex items-center justify-between">
        <label className="text-xs font-medium opacity-70">History</label>
        <div className="flex items-center gap-1.5">
          <select
            className="select select-xs"
            value={retentionDays}
            onChange={(e) => changeRetention(Number(e.target.value))}
          >
            {RETENTION_OPTIONS.map((days) => (
              <option key={days} value={days}>
                Keep {days}d
              </option>
            ))}
          </select>
          <button className="btn btn-ghost btn-xs" onClick={clear} disabled={entries.length === 0}>
            Clear
          </button>
        </div>
      </div>
      <label className="mb-1.5 flex items-center gap-1.5 text-xs opacity-80">
        <input
          type="checkbox"
          className="checkbox checkbox-xs"
          checked={journalEnabled}
          onChange={(e) => toggleJournal(e.target.checked)}
        />
        Summarize into a work journal (adds an LLM call per dictation)
      </label>
      <input
        className="input input-xs mb-1.5 w-full"
        placeholder="Search transcripts…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />
      {loading ? (
        <span className="loading loading-spinner loading-xs" />
      ) : entries.length === 0 ? (
        <p className="text-xs opacity-60">
          {query.trim() ? `No transcripts match "${query.trim()}".` : "No transcripts yet."}
        </p>
      ) : (
        <ul className="flex max-h-[420px] flex-col gap-1 overflow-y-auto rounded-md bg-base-100 p-1.5">
          {entries.map((entry) => {
            const reprocess = reprocessing[entry.timestamp_ms];
            const draft = editing[entry.timestamp_ms];
            const original = entry.original_text;
            const originalOpen = !!original && showOriginal.has(entry.timestamp_ms);
            const diffTokens =
              originalOpen && original && original.trim() !== entry.text.trim()
                ? computeWordDiff(original, entry.text)
                : null;
            const selected = diffSelection[entry.timestamp_ms] ?? null;
            const feedback = teachFeedback[entry.timestamp_ms];
            return (
              <li key={entry.timestamp_ms} className="group rounded px-1.5 py-1 text-xs hover:bg-base-200">
                <div className="flex items-center justify-between opacity-60">
                  <span className="flex items-center gap-1.5">
                    <span className="dw-mono text-[10.5px]">{formatTimestamp(entry.timestamp_ms)}</span>
                    {original && (
                      <button
                        className="badge badge-ghost badge-xs"
                        onClick={() => toggleOriginal(entry.timestamp_ms)}
                        aria-expanded={originalOpen}
                        title="Show the text before refinement"
                      >
                        Refined {originalOpen ? "▾" : "▸"}
                      </button>
                    )}
                  </span>
                  <div className="flex items-center gap-1.5">
                    {entry.app_name && <span className="truncate">{entry.app_name}</span>}
                    <button
                      className={`inline-flex items-center justify-center rounded p-0.5 transition-opacity hover:opacity-100 focus-visible:opacity-100 ${
                        draft !== undefined ? "opacity-100" : "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100"
                      }`}
                      onClick={() => (draft !== undefined ? stopEdit(entry.timestamp_ms) : startEdit(entry))}
                      aria-label="Edit transcript"
                      title="Edit"
                    >
                      <IconEdit />
                    </button>
                    <button
                      className={`inline-flex items-center justify-center rounded p-0.5 transition-opacity hover:opacity-100 focus-visible:opacity-100 ${
                        reprocess ? "opacity-100" : "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100"
                      }`}
                      onClick={() => toggleReprocess(entry.timestamp_ms)}
                      aria-label="Reprocess through a different mode"
                      title="Reprocess"
                    >
                      <IconReprocess />
                    </button>
                    <button
                      className="inline-flex items-center justify-center rounded p-0.5 opacity-0 transition-opacity focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100"
                      onClick={() => copyEntry(entry)}
                      aria-label="Copy transcript"
                      title="Copy"
                    >
                      {copiedAt === entry.timestamp_ms ? <IconCheck /> : <IconCopy />}
                    </button>
                    <button
                      className="inline-flex items-center justify-center rounded p-0.5 opacity-0 transition-opacity hover:text-error focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100"
                      onClick={() => deleteEntry(entry.timestamp_ms)}
                      aria-label="Delete transcript"
                      title="Delete"
                    >
                      <IconTrash />
                    </button>
                  </div>
                </div>
                {draft !== undefined ? (
                  <div className="mt-1 flex flex-col gap-1">
                    <textarea
                      className="textarea textarea-xs w-full"
                      rows={3}
                      value={draft}
                      aria-label="Edit transcript text"
                      onChange={(e) => setEditing((current) => ({ ...current, [entry.timestamp_ms]: e.target.value }))}
                    />
                    <div className="flex items-center gap-1.5">
                      <button className="btn btn-xs" onClick={() => saveEdit(entry)}>
                        Save
                      </button>
                      <button className="btn btn-ghost btn-xs" onClick={() => stopEdit(entry.timestamp_ms)}>
                        Cancel
                      </button>
                    </div>
                  </div>
                ) : entry.summary ? (
                  <>
                    <p className="truncate font-medium" title={entry.summary}>
                      {entry.summary}
                    </p>
                    <p className="truncate opacity-50" title={entry.text}>
                      {entry.text}
                    </p>
                  </>
                ) : (
                  <p className="truncate" title={entry.text}>
                    {entry.text}
                  </p>
                )}
                {originalOpen && (
                  <div className="mt-1.5 flex flex-col gap-1.5 rounded bg-base-200 p-2" data-testid="original-text">
                    {diffTokens && (
                      <div className="flex flex-col gap-1.5">
                        <div className="flex flex-wrap items-center justify-between gap-2">
                          <span className="text-[10px] font-semibold uppercase tracking-wide opacity-50">
                            Refinement diff
                          </span>
                          <span className="text-[10px] opacity-45">
                            Click any highlighted word to teach Vocabulary
                          </span>
                        </div>
                        <p className="whitespace-pre-wrap leading-relaxed">
                          {diffTokens.map((tok, idx) => {
                            if (tok.kind === "eq") {
                              return (
                                <span key={idx}>
                                  {idx > 0 ? " " : ""}
                                  {tok.text}
                                </span>
                              );
                            }
                            const cleaned = cleanVocabWord(tok.text);
                            const isSelected = selected?.tokenIdx === idx;
                            const isPairedSibling =
                              !isSelected &&
                              selected !== null &&
                              tok.pair !== undefined &&
                              diffTokens[selected.tokenIdx]?.pair === tok.pair;
                            const tokenClass = tok.kind === "del" ? "dw-diff-del" : "dw-diff-ins";
                            if (!cleaned) {
                              return (
                                <span key={idx}>
                                  {idx > 0 ? " " : ""}
                                  <span className={tokenClass}>{tok.text}</span>
                                </span>
                              );
                            }
                            return (
                              <span key={idx}>
                                {idx > 0 ? " " : ""}
                                <button
                                  type="button"
                                  className={`${tokenClass} dw-diff-clickable`}
                                  style={
                                    isSelected
                                      ? {
                                          boxShadow:
                                            "0 0 0 1.5px var(--dw-accent), 0 0 10px -2px color-mix(in srgb, var(--dw-accent) 55%, transparent)",
                                        }
                                      : isPairedSibling
                                        ? {
                                            boxShadow:
                                              "0 0 0 1px color-mix(in srgb, var(--dw-accent) 45%, transparent)",
                                          }
                                        : undefined
                                  }
                                  aria-pressed={isSelected}
                                  title={`Click to teach "${cleaned}" to Vocabulary`}
                                  onClick={() => selectDiffToken(entry.timestamp_ms, idx, tok)}
                                >
                                  {tok.text}
                                </button>
                              </span>
                            );
                          })}
                        </p>
                        {selected && (
                          <div
                            data-testid="diff-teach-bar"
                            className="flex flex-wrap items-center gap-1.5 rounded-md border border-base-content/10 bg-base-100/90 px-2 py-1.5"
                            style={{
                              boxShadow:
                                "inset 0 1px 0 var(--dw-specular, rgba(255,255,255,0.08)), 0 4px 12px -4px rgba(0,0,0,0.28)",
                            }}
                          >
                            {feedback?.vocabAdded === selected.word ? (
                              <span
                                className="inline-flex items-center gap-1 rounded px-2 py-0.5 text-[11px] font-medium"
                                style={{
                                  backgroundColor: "color-mix(in srgb, var(--color-success) 16%, transparent)",
                                  color: "var(--color-success)",
                                  border: "1px solid color-mix(in srgb, var(--color-success) 36%, transparent)",
                                }}
                              >
                                ✓ Added &quot;{selected.word}&quot; to Vocabulary
                              </span>
                            ) : (
                              <button
                                type="button"
                                className="btn btn-xs h-6 min-h-0 gap-1 border font-medium"
                                style={{
                                  borderColor: "color-mix(in srgb, var(--dw-accent) 45%, transparent)",
                                  backgroundColor: "color-mix(in srgb, var(--dw-accent) 14%, transparent)",
                                  color: "var(--dw-accent)",
                                }}
                                disabled={feedback?.busyVocab}
                                onClick={() => void teachVocabulary(entry.timestamp_ms, selected.word)}
                              >
                                Add <span className="dw-mono font-semibold">{selected.word}</span> to Vocabulary
                              </button>
                            )}
                            {selected.pair &&
                              (feedback?.ruleSaved === `${selected.pair.from} → ${selected.pair.to}` ? (
                                <span
                                  className="inline-flex items-center gap-1 rounded px-2 py-0.5 text-[11px] font-medium"
                                  style={{
                                    backgroundColor: "color-mix(in srgb, var(--dw-refine) 18%, transparent)",
                                    color: "var(--dw-refine-bright)",
                                    border: "1px solid color-mix(in srgb, var(--dw-refine) 40%, transparent)",
                                  }}
                                >
                                  ✓ Rule saved
                                </span>
                              ) : (
                                <button
                                  type="button"
                                  className="btn btn-xs h-6 min-h-0 gap-1 border font-medium"
                                  style={{
                                    borderColor: "color-mix(in srgb, var(--dw-refine) 45%, transparent)",
                                    backgroundColor: "color-mix(in srgb, var(--dw-refine) 14%, transparent)",
                                    color: "var(--dw-refine-bright)",
                                  }}
                                  disabled={feedback?.busyRule}
                                  onClick={() =>
                                    void teachReplacement(entry.timestamp_ms, selected.pair!.from, selected.pair!.to)
                                  }
                                >
                                  Always replace <span className="dw-mono font-semibold">{selected.pair.from}</span> →{" "}
                                  <span className="dw-mono font-semibold">{selected.pair.to}</span>
                                </button>
                              ))}
                            <button
                              type="button"
                              className="btn btn-ghost btn-xs ml-auto h-5 min-h-0 px-1 text-[11px] opacity-60 hover:opacity-100"
                              aria-label="Dismiss teach bar"
                              onClick={() => clearDiffSelection(entry.timestamp_ms)}
                            >
                              ✕
                            </button>
                          </div>
                        )}
                      </div>
                    )}
                    <div
                      className={
                        diffTokens ? "flex flex-col gap-0.5 border-t border-base-content/10 pt-1.5" : "flex flex-col gap-0.5"
                      }
                    >
                      <span className="text-[10px] font-medium opacity-50">Original, before refinement</span>
                      <p className="whitespace-pre-wrap opacity-75">{original}</p>
                    </div>
                    <div className="flex items-center gap-1.5">
                      <button className="btn btn-ghost btn-xs" onClick={() => copyOriginal(entry)}>
                        Copy original
                      </button>
                      <button className="btn btn-ghost btn-xs" onClick={() => useOriginal(entry)}>
                        Use original
                      </button>
                    </div>
                  </div>
                )}
                {reprocess && (
                  <div className="mt-1 flex flex-col gap-1 rounded bg-base-200 p-1.5">
                    <div className="flex items-center gap-1.5">
                      <select
                        className="select select-xs"
                        value={reprocess.mode}
                        onChange={(e) => patchReprocess(entry.timestamp_ms, { mode: e.target.value as Behavior })}
                      >
                        {BEHAVIORS.map((m) => (
                          <option key={m} value={m}>
                            {BEHAVIOR_LABEL[m]}
                          </option>
                        ))}
                      </select>
                      <label className="flex items-center gap-1 opacity-80">
                        <input
                          type="checkbox"
                          className="checkbox checkbox-xs"
                          checked={reprocess.useLlm}
                          onChange={(e) => patchReprocess(entry.timestamp_ms, { useLlm: e.target.checked })}
                        />
                        Refine with LLM
                      </label>
                      <button
                        className="btn btn-xs ml-auto"
                        onClick={() => runReprocess(entry)}
                        disabled={reprocess.loading}
                      >
                        {reprocess.loading ? <span className="loading loading-spinner loading-xs" /> : "Run"}
                      </button>
                    </div>
                    {reprocess.error && <p className="text-error">{reprocess.error}</p>}
                    {reprocess.result && (
                      <>
                        <p className="whitespace-pre-wrap rounded bg-base-100 p-1.5" title={reprocess.result}>
                          {reprocess.result}
                        </p>
                        <div className="flex items-center gap-1.5">
                          <button
                            className="btn btn-ghost btn-xs"
                            onClick={() => navigator.clipboard.writeText(reprocess.result ?? "")}
                          >
                            Copy result
                          </button>
                          <button className="btn btn-ghost btn-xs" onClick={() => replaceWithReprocessed(entry)}>
                            Replace
                          </button>
                          <button
                            className="btn btn-ghost btn-xs ml-auto"
                            onClick={() => toggleReprocess(entry.timestamp_ms)}
                          >
                            Discard
                          </button>
                        </div>
                      </>
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

// History, behind the optional lock (see privacy.tsx and privacy.rs).
export function HistorySection() {
  const lock = useHistoryLock();
  return (
    <>
      <HistoryPrivacy lock={lock} />
      {lock.status === null ? (
        <span className="loading loading-spinner loading-xs" />
      ) : lock.status.locked ? (
        <LockedHistory lock={lock} />
      ) : (
        <HistoryContent />
      )}
    </>
  );
}
