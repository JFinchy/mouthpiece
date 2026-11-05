

export type ModelStatus = {
  id: string;
  label: string;
  size_mb: number;
  downloaded: boolean;
  active: boolean;
};

export type Behavior = "plain" | "casual" | "cli";

export const BEHAVIORS: Behavior[] = ["plain", "casual", "cli"];

export const BEHAVIOR_LABEL: Record<Behavior, string> = { plain: "Plain", casual: "Casual", cli: "CLI" };

export type LlmModelStatus = {
  id: string;
  label: string;
  size_gb: number;
  downloaded: boolean;
  last_latency_ms: number | null;
  /** What runs it: "Ollama", "LM Studio", "llama.cpp server", "Jan". */
  source: string;
};

/** A `.gguf` file on this Mac that can be added to Ollama (local_llm.rs). */
export type ModelFile = {
  path: string;
  file_name: string;
  size_gb: number;
  location: string;
  ollama_name: string;
  in_ollama: boolean;
};

export type WidgetMode = "minimal" | "compact" | "detailed" | "pill" | "notch" | "caption" | "stacked";
