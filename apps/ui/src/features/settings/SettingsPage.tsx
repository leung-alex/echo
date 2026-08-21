import { useEffect, useState, type MouseEvent, type ReactElement } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { EchoIcon } from "../../ui/icons/EchoIcon";
import { Switch } from "../../ui/Switch";
import {
  clearHistory,
  getSettings,
  type ClipboardSettings,
  updateSettings,
} from "./api";

export function SettingsPage({ onBack }: { onBack: () => void }): ReactElement {
  const [settings, setSettings] = useState<ClipboardSettings | null>(null);
  const [status, setStatus] = useState("Loading");
  const [statusKind, setStatusKind] = useState<"info" | "success" | "error">(
    "info",
  );

  const report = (
    message: string,
    kind: "info" | "success" | "error" = "info",
  ) => {
    setStatus(message);
    setStatusKind(kind);
  };

  const reload = async () => {
    try {
      setSettings(await getSettings());
      report("Ready");
    } catch (error) {
      report(error instanceof Error ? error.message : String(error), "error");
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  const save = async () => {
    if (!settings) return;
    try {
      await updateSettings(settings);
      report("Settings updated", "success");
    } catch (error) {
      report(error instanceof Error ? error.message : String(error), "error");
    }
  };

  const startHeaderDrag = (event: MouseEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    const target = event.target;
    if (
      target instanceof Element &&
      target.closest(
        'button, input, textarea, select, a, [contenteditable="true"]',
      )
    ) {
      return;
    }
    event.preventDefault();
    void getCurrentWindow()
      .startDragging()
      .catch(() => undefined);
  };

  return (
    <main className="clipboard-settings-window">
      <header className="settings-header" onMouseDown={startHeaderDrag}>
        <button
          className="echo-icon-button"
          type="button"
          aria-label="Back"
          title="Back"
          onClick={onBack}
        >
          <EchoIcon name="back" size={17} aria-hidden="true" />
        </button>
        <div>
          <h1>Clipboard Settings</h1>
          <p>Recording, privacy, and storage capacity.</p>
        </div>
      </header>
      <div className="clipboard-settings-columns">
        <section
          className="settings-column"
          aria-labelledby="recording-heading"
        >
          <h2 id="recording-heading">Recording and privacy</h2>
          {settings ? (
            <>
              <SettingToggle
                label="Record clipboard history"
                checked={settings.history_enabled}
                onChange={() =>
                  setSettings({
                    ...settings,
                    history_enabled: !settings.history_enabled,
                  })
                }
              />
              <SettingToggle
                label="Record sensitive content"
                checked={settings.record_sensitive}
                onChange={() =>
                  setSettings({
                    ...settings,
                    record_sensitive: !settings.record_sensitive,
                  })
                }
              />
              <SettingToggle
                label="Store source window titles"
                checked={settings.store_window_titles}
                onChange={() =>
                  setSettings({
                    ...settings,
                    store_window_titles: !settings.store_window_titles,
                  })
                }
              />
              <p className="privacy-notice">
                Echo stores only the representations needed for History and
                Quick Insert.
              </p>
            </>
          ) : null}
        </section>
        <section className="settings-column" aria-labelledby="capacity-heading">
          <h2 id="capacity-heading">Capacity</h2>
          {settings ? (
            <>
              <NumberSetting
                label="Maximum entries"
                value={settings.max_entries}
                onChange={(value) =>
                  setSettings({ ...settings, max_entries: value })
                }
              />
              <NumberSetting
                label="Total storage (MB)"
                value={Math.max(
                  1,
                  Math.round(settings.max_total_bytes / 1024 / 1024),
                )}
                onChange={(value) =>
                  setSettings({
                    ...settings,
                    max_total_bytes: value * 1024 * 1024,
                  })
                }
              />
              <NumberSetting
                label="Single item (MB)"
                value={Math.max(
                  1,
                  Math.round(settings.max_item_bytes / 1024 / 1024),
                )}
                onChange={(value) =>
                  setSettings({
                    ...settings,
                    max_item_bytes: value * 1024 * 1024,
                  })
                }
              />
              <div className="clipboard-settings-actions">
                <button
                  className="primary-action"
                  type="button"
                  onClick={() => void save()}
                >
                  Save settings
                </button>
                <button
                  className="danger"
                  type="button"
                  onClick={() => {
                    if (window.confirm("Clear clipboard history?"))
                      void clearHistory()
                        .then(() => report("History cleared", "success"))
                        .catch((error) => report(String(error), "error"));
                  }}
                >
                  Clear history
                </button>
              </div>
            </>
          ) : null}
        </section>
      </div>
      <p
        className="clipboard-settings-status"
        data-kind={statusKind}
        role={statusKind === "error" ? "alert" : "status"}
      >
        {status}
      </p>
    </main>
  );
}

function SettingToggle({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: () => void;
}) {
  return (
    <div className="setting-toggle">
      <span>{label}</span>
      <Switch aria-label={label} checked={checked} onCheckedChange={onChange} />
    </div>
  );
}

function NumberSetting({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="number-setting">
      <span>{label}</span>
      <input
        type="number"
        min="1"
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    </label>
  );
}
