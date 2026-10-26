import { useState, type ReactElement } from "react";
import { ReadinessPanel } from "./ReadinessPanel";
import { type ReadinessFix } from "./readiness";
import { IconAgent, IconBook, IconChart, IconClock, IconCpu, IconGear, IconInfo, IconMic, IconPalette, IconPlug, IconWave, IconWindow } from "./icons";
import { AgentsSection } from "./sections/agents";
import { AboutSection, LogsSection, UpdatesSection } from "./sections/app";
import { DeviceSection, GeneralSection, ShortcutSection, VoiceCommandsSection } from "./sections/dictation";
import { HistorySection } from "./sections/history";
import { InsightsSection } from "./sections/insights";
import { DeliverySection, WebhookSection } from "./sections/integrations";
import { LlmSection } from "./sections/llm";
import { AppModesSection } from "./sections/modes";
import { SnippetsSection, VocabularySection } from "./sections/vocabulary";
import { ModelsSection, VoiceIsolationSection } from "./sections/voice";

export type PageId =
  | "dictation"
  | "voice"
  | "vocabulary"
  | "modes"
  | "llm"
  | "insights"
  | "history"
  | "integrations"
  | "agents"
  | "appearance"
  | "about"
  | "advanced";

export const NAV: { id: PageId; label: string; icon: () => ReactElement }[] = [
  { id: "dictation", label: "Dictation", icon: IconMic },
  { id: "voice", label: "Voice", icon: IconWave },
  { id: "vocabulary", label: "Vocabulary", icon: IconBook },
  { id: "modes", label: "Modes", icon: IconWindow },
  { id: "llm", label: "LLM", icon: IconCpu },
  { id: "insights", label: "Insights", icon: IconChart },
  { id: "history", label: "History", icon: IconClock },
  { id: "integrations", label: "Integrations", icon: IconPlug },
  { id: "agents", label: "Coding agents", icon: IconAgent },
];

export const FOOTER_NAV: { id: PageId; label: string; icon: () => ReactElement }[] = [
  { id: "appearance", label: "Appearance", icon: IconPalette },
  { id: "about", label: "About", icon: IconInfo },
  { id: "advanced", label: "Advanced", icon: IconGear },
];

export const PAGE_TITLE: Record<PageId, string> = {
  dictation: "Dictation",
  voice: "Voice — Whisper model & isolation",
  vocabulary: "Vocabulary",
  modes: "Modes — app-aware formatting rules",
  llm: "LLM — local refinement (Ollama)",
  insights: "Insights — usage & feature adoption",
  history: "History",
  integrations: "Integrations — delivery & webhook",
  agents: "Coding agents",
  appearance: "Appearance",
  about: "About — features & how it works",
  advanced: "Advanced",
};

export function NavItem({
  label,
  icon,
  active,
  onClick,
}: {
  label: string;
  icon: ReactElement;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button type="button" className={`dw-nav-item ${active ? "active" : ""}`} onClick={onClick}>
      <span className="dw-icon">{icon}</span>
      {label}
    </button>
  );
}

// Where each in-app readiness fix lives, per layout.
export const FIX_PAGE: Partial<Record<ReadinessFix, PageId>> = {
  download_model: "voice",
  change_shortcut: "dictation",
  choose_microphone: "dictation",
  pull_llm_model: "llm",
};

export const FIX_NODE: Partial<Record<ReadinessFix, NodeId>> = {
  download_model: "recognition",
  change_shortcut: "input",
  choose_microphone: "input",
  pull_llm_model: "refinement",
};

export function SidebarLayout({ appearance }: { appearance: ReactElement }) {
  const [page, setPage] = useState<PageId>("dictation");

  return (
    <main className="dw-shell h-screen">
      <div className="dw-app-shell">
        <nav className="dw-sidebar">
          <div className="dw-brand">
            <IconMic />
            Mouthpiece
          </div>
          {NAV.map((item) => (
            <NavItem
              key={item.id}
              label={item.label}
              icon={item.icon()}
              active={page === item.id}
              onClick={() => setPage(item.id)}
            />
          ))}
          <div className="dw-sidebar-foot">
            {FOOTER_NAV.map((item) => (
              <NavItem
                key={item.id}
                label={item.label}
                icon={item.icon()}
                active={page === item.id}
                onClick={() => setPage(item.id)}
              />
            ))}
          </div>
        </nav>

        <div className="dw-page">
          <h2 className="dw-page-title">{PAGE_TITLE[page]}</h2>
          <div className="dw-page-body">
            <ReadinessPanel onNavigate={(fix) => setPage(FIX_PAGE[fix] ?? page)} />
            {page === "dictation" && (
              <>
                <GeneralSection />
                <DeviceSection />
                <ShortcutSection />
                <VoiceCommandsSection />
              </>
            )}
            {page === "voice" && (
              <>
                <ModelsSection />
                <VoiceIsolationSection />
              </>
            )}
            {page === "vocabulary" && (
              <>
                <VocabularySection />
                <SnippetsSection />
              </>
            )}
            {page === "modes" && <AppModesSection />}
            {page === "llm" && <LlmSection />}
            {page === "insights" && <InsightsSection />}
            {page === "history" && <HistorySection />}
            {page === "integrations" && (
              <>
                <DeliverySection />
                <WebhookSection />
              </>
            )}
            {page === "agents" && <AgentsSection />}
            {page === "appearance" && appearance}
            {page === "about" && (
              <>
                <UpdatesSection />
                <AboutSection />
              </>
            )}
            {page === "advanced" && <LogsSection />}
          </div>
        </div>
      </div>
    </main>
  );
}

export type NodeId = "input" | "recognition" | "mode" | "refinement" | "output";

export type ActiveNode = NodeId | "app" | null;

export const CHAIN: { id: NodeId; label: string; icon: () => ReactElement }[] = [
  { id: "input", label: "Input", icon: IconMic },
  { id: "recognition", label: "Recognition", icon: IconWave },
  { id: "mode", label: "Mode", icon: IconWindow },
  { id: "refinement", label: "Refinement", icon: IconCpu },
  { id: "output", label: "Output", icon: IconPlug },
];

export const NODE_DRAWER_TITLE: Record<NodeId, string> = {
  input: "Input — microphone & shortcut",
  recognition: "Recognition — Whisper model, voice isolation, vocabulary",
  mode: "Mode — app-aware formatting rules",
  refinement: "Refinement — local LLM (Ollama)",
  output: "Output — delivery, webhook, history & insights",
};

export function ChainNode({
  label,
  icon,
  active,
  onClick,
}: {
  label: string;
  icon: ReactElement;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button type="button" className={`dw-chain-node ${active ? "active" : ""}`} onClick={onClick}>
      <span className="dw-icon">{icon}</span>
      <span className="dw-label">{label}</span>
    </button>
  );
}

export function ChainLayout({ appearance }: { appearance: ReactElement }) {
  const [active, setActive] = useState<ActiveNode>("input");

  function toggle(id: ActiveNode) {
    setActive((current) => (current === id ? null : id));
  }

  return (
    <main className="dw-shell h-screen overflow-y-auto">
      <div className="dw-titlebar">
        <h1>Mouthpiece — signal chain</h1>
      </div>

      <div className="dw-chain-readiness">
        <ReadinessPanel onNavigate={(fix) => setActive(FIX_NODE[fix] ?? active)} />
      </div>

      <div className="dw-chain-row">
        {CHAIN.map((node) => (
          <div key={node.id} className="contents">
            <ChainNode
              label={node.label}
              icon={node.icon()}
              active={active === node.id}
              onClick={() => toggle(node.id)}
            />
            {node.id !== "output" && <div className="dw-chain-link" />}
          </div>
        ))}
        <div className="dw-chain-app-gap" />
        <ChainNode label="App" icon={<IconGear />} active={active === "app"} onClick={() => toggle("app")} />
      </div>

      {active && (
        <div className="dw-drawer">
          <p className="dw-drawer-title">
            {active === "app" ? "App — general, appearance & advanced" : NODE_DRAWER_TITLE[active]}
          </p>
          {active === "input" && (
            <>
              <DeviceSection />
              <ShortcutSection />
            </>
          )}
          {active === "recognition" && (
            <>
              <ModelsSection />
              <VoiceIsolationSection />
              <VocabularySection />
              <SnippetsSection />
            </>
          )}
          {active === "mode" && <AppModesSection />}
          {active === "refinement" && <LlmSection />}
          {active === "output" && (
            <>
              <DeliverySection />
              <WebhookSection />
              <div className="mb-4 border-t border-base-content/10 pt-3">
                <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide opacity-60">Coding agents</h3>
                <AgentsSection />
              </div>
              <HistorySection />
              <div className="mb-4 border-t border-base-content/10 pt-3">
                <InsightsSection />
              </div>
            </>
          )}
          {active === "app" && (
            <>
              <GeneralSection />
              {appearance}
              <UpdatesSection />
              <div className="mb-4 border-t border-base-content/10 pt-3">
                <AboutSection />
              </div>
              <LogsSection />
            </>
          )}
        </div>
      )}
    </main>
  );
}
