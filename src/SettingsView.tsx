import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { applyTheme, type ThemeId } from "./theme";
import { notifyError, Toaster } from "./ui/toast";
import { ChainLayout, SidebarLayout, useSettingsNavigation } from "./settings/layouts";
import { type LayoutId, LayoutSection, ThemeSection } from "./settings/sections/app";

function SettingsView() {
  const [theme, setThemeState] = useState<ThemeId>("terminal");
  const [layout, setLayoutState] = useState<LayoutId>("sidebar");
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

    const unlistenTheme = listen<ThemeId>("theme-changed", (e) => {
      setThemeState(e.payload);
      applyTheme(e.payload);
    });
    const unlistenLayout = listen<LayoutId>("layout-changed", (e) => setLayoutState(e.payload));
    return () => {
      unlistenTheme.then((f) => f());
      unlistenLayout.then((f) => f());
    };
  }, []);

  function changeTheme(next: ThemeId) {
    setThemeState(next);
    applyTheme(next);
    invoke("set_theme", { theme: next }).catch((err) => notifyError("set_theme", err));
  }

  function changeLayout(next: LayoutId) {
    setLayoutState(next);
    invoke("set_layout", { layout: next }).catch((err) => notifyError("set_layout", err));
  }

  const appearance = (
    <>
      <ThemeSection theme={theme} onChange={changeTheme} />
      <LayoutSection layout={layout} onChange={changeLayout} />
    </>
  );

  return (
    <>
      {layout === "chain" ? (
        <ChainLayout appearance={appearance} focusMeetingId={focusMeetingId} focusNonce={focusNonce} />
      ) : (
        <SidebarLayout appearance={appearance} focusMeetingId={focusMeetingId} focusNonce={focusNonce} />
      )}
      <Toaster />
    </>
  );
}

export default SettingsView;
