import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { applyTheme, THEME_ORDER, type ThemeId } from "./theme";
import { DEFAULT_WAVEFORM_CONFIG, type WaveformConfig } from "./waveformConfig";
import { notifyError, Toaster } from "./ui/toast";
import { ChainLayout, SidebarLayout, useSettingsNavigation } from "./settings/layouts";
import { type LayoutId, LayoutSection, ThemeSection, WaveformSection } from "./settings/sections/app";

function SettingsView() {
  const [theme, setThemeState] = useState<ThemeId>("terminal");
  const [layout, setLayoutState] = useState<LayoutId>("sidebar");
  const [waveform, setWaveformState] = useState<WaveformConfig>(DEFAULT_WAVEFORM_CONFIG);
  const [focusMeetingId, setFocusMeetingId] = useState<string | null>(null);
  const [focusNonce, setFocusNonce] = useState(0);

  useSettingsNavigation((nav) => {
    if (nav.page === "meetings") {
      setFocusMeetingId(nav.meeting_id ?? "latest");
      setFocusNonce((n) => n + 1);
    }
  });

  useEffect(() => {
    invoke<ThemeId>("get_theme")
      .then((t) => {
        setThemeState(t);
        applyTheme(t);
      })
      .catch((err) => notifyError("get_theme", err));

    invoke<LayoutId>("get_layout")
      .then(setLayoutState)
      .catch((err) => notifyError("get_layout", err));

    invoke<WaveformConfig>("get_waveform_config")
      .then((cfg) => setWaveformState({ ...DEFAULT_WAVEFORM_CONFIG, ...cfg }))
      .catch((err) => notifyError("get_waveform_config", err));

    const unlistenTheme = listen<ThemeId>("theme-changed", (e) => {
      setThemeState(e.payload);
      applyTheme(e.payload);
    });
    const unlistenLayout = listen<LayoutId>("layout-changed", (e) => setLayoutState(e.payload));
    const unlistenWaveform = listen<WaveformConfig>("waveform-config-changed", (e) => {
      setWaveformState({ ...DEFAULT_WAVEFORM_CONFIG, ...e.payload });
    });
    return () => {
      unlistenTheme.then((f) => f());
      unlistenLayout.then((f) => f());
      unlistenWaveform.then((f) => f());
    };
  }, []);

  // Quick ⌘1–⌘4 theme switching matching the hardware keycap badges in Appearance
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.isContentEditable)
      ) {
        return;
      }
      const idx = Number(e.key) - 1;
      if (idx >= 0 && idx < THEME_ORDER.length) {
        const next = THEME_ORDER[idx];
        e.preventDefault();
        setThemeState(next);
        applyTheme(next);
        invoke("set_theme", { theme: next }).catch((err) => notifyError("set_theme", err));
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // Once the Meetings list has opened the deep-linked meeting, forget it,
  // so coming back to Meetings later doesn't open it again.
  function clearFocusMeeting() {
    setFocusMeetingId(null);
  }

  function changeTheme(next: ThemeId) {
    setThemeState(next);
    applyTheme(next);
    invoke("set_theme", { theme: next }).catch((err) => notifyError("set_theme", err));
  }

  function changeLayout(next: LayoutId) {
    setLayoutState(next);
    invoke("set_layout", { layout: next }).catch((err) => notifyError("set_layout", err));
  }

  const waveformSeqRef = useRef(0);

  function changeWaveform(next: WaveformConfig) {
    const seq = ++waveformSeqRef.current;
    const previous = waveform;
    setWaveformState(next);
    invoke("set_waveform_config", { config: next }).catch((err) => {
      notifyError("set_waveform_config", err);
      if (seq === waveformSeqRef.current) {
        setWaveformState(previous);
      }
    });
  }

  const appearance = (
    <>
      <WaveformSection config={waveform} onChange={changeWaveform} />
      <ThemeSection theme={theme} onChange={changeTheme} />
      <LayoutSection layout={layout} onChange={changeLayout} />
    </>
  );

  return (
    <>
      {layout === "chain" ? (
        <ChainLayout
          appearance={appearance}
          waveform={waveform}
          onChangeWaveform={changeWaveform}
          focusMeetingId={focusMeetingId}
          focusNonce={focusNonce}
          onFocusHandled={clearFocusMeeting}
        />
      ) : (
        <SidebarLayout
          appearance={appearance}
          waveform={waveform}
          onChangeWaveform={changeWaveform}
          focusMeetingId={focusMeetingId}
          focusNonce={focusNonce}
          onFocusHandled={clearFocusMeeting}
        />
      )}
      <Toaster />
    </>
  );
}

export default SettingsView;
