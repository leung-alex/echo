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
import { invoke } from "../shared/ipc/invoke";

type Route = ActivationPayload["route"];

export function EchoApp(): ReactElement {
  const [route, setRoute] = useState<Route>("history");
  const [query, setQuery] = useState("");
  const [session, setSession] = useState<PasteSession | null>(null);
  const [focusRequest, setFocusRequest] = useState(0);
  const [surfaceVersion, setSurfaceVersion] = useState(0);
  const handled = useRef(new Set<string>());

  const hideWindow = useCallback(async () => {
    try {
      await getCurrentWindow().hide();
    } catch {
      window.close();
    }
  }, []);

  const applyActivation = useCallback(async (payload: ActivationPayload) => {
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
        await invoke("activation_ack", {
          request_id: payload.request_id,
        }).catch(() => undefined);
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<ActivationPayload>("echo-activation", ({ payload }) => {
      void applyActivation(payload);
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    void invoke<ActivationPayload | null>("activation_state")
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

  if (route === "settings") return <SettingsPage onBack={backToHistory} />;
  return (
    <QuickInsertSurface
      key={`${route}:${surfaceVersion}`}
      initialSession={session}
      initialQuery={query}
      focusRequest={focusRequest}
      onClose={hideWindow}
      onOpenSettings={openSettings}
    />
  );
}
