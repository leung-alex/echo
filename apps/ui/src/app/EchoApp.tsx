import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactElement,
} from "react";
import { flushSync } from "react-dom";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { QuickInsertSurface } from "../features/quick-insert/QuickInsertSurface";
import type { PasteSession } from "../features/quick-insert/model/types";
import { SettingsPage } from "../features/settings/SettingsPage";
import type { ActivationPayload } from "../shared/ipc/generated";
import type { ThemeChangedEvent } from "../shared/ipc/generated";
import {
  acknowledgeActivation,
  getActivationState,
} from "../shared/ipc/activation";
import { getSettings } from "../features/settings/api";
import type { WindowRole } from "../features/quick-insert/model/types";

type Route = ActivationPayload["route"];

export function EchoApp(): ReactElement {
  const [windowRole] = useState<WindowRole>(() => getWindowRole());
  const [route, setRoute] = useState<Route>("history");
  const [query, setQuery] = useState("");
  const [session, setSession] = useState<PasteSession | null>(null);
  const [focusRequest, setFocusRequest] = useState(0);
  const [surfaceVersion, setSurfaceVersion] = useState(0);
  const handled = useRef(new Set<string>());

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const applyTheme = (
      mode: ThemeChangedEvent["mode"],
      nativeMica: boolean,
    ) => {
      if (disposed) return;
      if (mode === "system") delete document.documentElement.dataset.theme;
      else document.documentElement.dataset.theme = mode;
      document.documentElement.dataset.mica = nativeMica
        ? "native"
        : "fallback";
    };

    void getSettings()
      .then((settings) => applyTheme(settings.theme, false))
      .catch(() => undefined);
    void listen<ThemeChangedEvent>("echo-theme-changed", ({ payload }) => {
      applyTheme(payload.mode, payload.nativeMica);
    })
      .then((cleanup) => {
        if (disposed) cleanup();
        else unlisten = cleanup;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const hideWindow = useCallback(async () => {
    try {
      await getCurrentWindow().hide();
    } catch {
      window.close();
    }
  }, []);

  const applyActivation = useCallback(
    async (payload: ActivationPayload) => {
      if (windowRole === "favorites") return;
      if (payload.request_id && handled.current.has(payload.request_id)) return;
      if (payload.request_id) handled.current.add(payload.request_id);
      try {
        // Native activation captures the target before showing Echo. Capturing
        // again from the WebView would observe Echo itself and lose the target.
        flushSync(() => {
          setSession(null);
          setRoute(payload.route);
          setQuery(payload.query ?? "");
          setSurfaceVersion((value) => value + 1);
          setFocusRequest((value) => value + 1);
        });
      } finally {
        if (payload.request_id)
          await acknowledgeActivation(payload.request_id).catch(
            () => undefined,
          );
      }
    },
    [windowRole],
  );

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<ActivationPayload>("echo-activation", ({ payload }) => {
      void applyActivation(payload);
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    void getActivationState()
      .then((pending) => {
        if (pending) void applyActivation(pending);
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [applyActivation]);

  const openSettings = () => {
    setRoute("settings");
    setSession(null);
    setFocusRequest((value) => value + 1);
  };
  const backToHistory = () => {
    setRoute("history");
    setQuery("");
    setFocusRequest((value) => value + 1);
  };

  if (windowRole === "favorites") {
    return (
      <QuickInsertSurface
        key={`favorites:${surfaceVersion}`}
        initialView="favorites"
        runtimeContext="manager"
        windowRole="favorites"
        showPanelTabs={false}
        onClose={hideWindow}
      />
    );
  }
  if (route === "settings") return <SettingsPage onBack={backToHistory} />;
  return (
    <QuickInsertSurface
      key={`${route}:${surfaceVersion}`}
      initialSession={session}
      initialQuery={query}
      focusRequest={focusRequest}
      runtimeContext={route === "quick_insert" ? "quick-insert" : "manager"}
      windowRole="main"
      showPanelTabs
      onClose={hideWindow}
      onOpenSettings={openSettings}
    />
  );
}

function getWindowRole(): WindowRole {
  try {
    return getCurrentWindow().label === "favorites" ? "favorites" : "main";
  } catch {
    return "main";
  }
}
