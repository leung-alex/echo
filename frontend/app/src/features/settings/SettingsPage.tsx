import { useEffect, useState, type ReactElement } from "react";
import { ArrowLeft, Plus, Pencil, Trash2 } from "lucide-react";

import { Switch } from "../../ui/Switch";
import { quickInsertClient, type EchoSnippet, type QuickInsertClient } from "../quick-insert/api/quick-insert-client";
import { clearHistory, getSettings, type ClipboardSettings, updateSettings } from "./api";

export function SettingsPage({ onBack, client = quickInsertClient }: { onBack: () => void; client?: QuickInsertClient }): ReactElement {
  const [settings, setSettings] = useState<ClipboardSettings | null>(null);
  const [snippets, setSnippets] = useState<EchoSnippet[]>([]);
  const [status, setStatus] = useState("Loading");
  const [statusKind, setStatusKind] = useState<"info" | "success" | "error">("info");
  const [editing, setEditing] = useState<number | null>(null);
  const [formOpen, setFormOpen] = useState(false);
  const [form, setForm] = useState({ name: "", content: "", group_name: "" });

  const report = (message: string, kind: "info" | "success" | "error" = "info") => { setStatus(message); setStatusKind(kind); };
  const reload = async () => {
    try {
      const [nextSettings, nextSnippets] = await Promise.all([getSettings(), client.listSnippets("")]);
      setSettings(nextSettings);
      setSnippets(nextSnippets);
      report("Ready");
    } catch (error) { report(error instanceof Error ? error.message : String(error), "error"); }
  };
  useEffect(() => { void reload(); }, []);

  const save = async () => {
    if (!settings) return;
    try { await updateSettings(settings); report("Settings updated", "success"); }
    catch (error) { report(error instanceof Error ? error.message : String(error), "error"); }
  };
  const saveSnippet = async () => {
    if (!form.name.trim() || !form.content.trim()) { report("Snippet name and content are required", "error"); return; }
    try { await client.saveSnippet(editing, form.name.trim(), form.content, form.group_name.trim() || null); setForm({ name: "", content: "", group_name: "" }); setEditing(null); setFormOpen(false); setSnippets(await client.listSnippets("")); report("Snippet saved", "success"); }
    catch (error) { report(error instanceof Error ? error.message : String(error), "error"); }
  };
  const removeSnippet = async (snippet: EchoSnippet) => {
    if (!window.confirm(`Delete snippet “${snippet.name}”?`)) return;
    try { await client.deleteSnippet(snippet.id); setSnippets(await client.listSnippets("")); report("Snippet deleted", "success"); }
    catch (error) { report(error instanceof Error ? error.message : String(error), "error"); }
  };
  const editSnippet = (snippet: EchoSnippet) => { setEditing(snippet.id); setFormOpen(true); setForm({ name: snippet.name, content: snippet.content, group_name: snippet.group_name ?? "" }); };

  return (
    <main className="clipboard-settings-window">
      <header className="settings-header"><button type="button" aria-label="Back" title="Back" onClick={onBack}><ArrowLeft size={17} aria-hidden="true" /></button><div><h1>Clipboard Settings</h1><p>Recording, privacy, and reusable snippets.</p></div></header>
      <div className="clipboard-settings-columns">
        <section className="settings-column" aria-labelledby="recording-heading"><h2 id="recording-heading">Recording and privacy</h2>
          {settings ? <>
            <SettingToggle label="Record clipboard history" checked={settings.history_enabled} onChange={() => setSettings({ ...settings, history_enabled: !settings.history_enabled })} />
            <SettingToggle label="Record sensitive content" checked={settings.record_sensitive} onChange={() => setSettings({ ...settings, record_sensitive: !settings.record_sensitive })} />
            <SettingToggle label="Store source window titles" checked={settings.store_window_titles} onChange={() => setSettings({ ...settings, store_window_titles: !settings.store_window_titles })} />
            <p className="privacy-notice">Echo stores only the representations needed for History and Quick Insert.</p>
          </> : null}
        </section>
        <section className="settings-column" aria-labelledby="capacity-heading"><h2 id="capacity-heading">Capacity</h2>
          {settings ? <>
            <NumberSetting label="Maximum entries" value={settings.max_entries} onChange={(value) => setSettings({ ...settings, max_entries: value })} />
            <NumberSetting label="Total storage (MB)" value={Math.max(1, Math.round(settings.max_total_bytes / 1024 / 1024))} onChange={(value) => setSettings({ ...settings, max_total_bytes: value * 1024 * 1024 })} />
            <NumberSetting label="Single item (MB)" value={Math.max(1, Math.round(settings.max_item_bytes / 1024 / 1024))} onChange={(value) => setSettings({ ...settings, max_item_bytes: value * 1024 * 1024 })} />
            <div className="clipboard-settings-actions"><button className="primary-action" type="button" onClick={() => void save()}>Save settings</button><button className="danger" type="button" onClick={() => { if (window.confirm("Clear clipboard history?")) void clearHistory().then(() => report("History cleared", "success")).catch((error) => report(String(error), "error")); }}>Clear history</button></div>
          </> : null}
        </section>
      </div>
      <section className="settings-snippets" aria-labelledby="snippets-heading"><div className="settings-section-heading"><div><h2 id="snippets-heading">Snippets</h2><p>Reusable text available in Quick Insert.</p></div><button type="button" onClick={() => { setEditing(null); setFormOpen(true); setForm({ name: "", content: "", group_name: "" }); }}><Plus size={15} aria-hidden="true" /> New snippet</button></div>
        {formOpen ? <div className="snippet-form"><h3>{editing === null ? "New snippet" : "Edit snippet"}</h3><label>Snippet name<input aria-label="Snippet name" value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} /></label><label>Snippet group<input aria-label="Snippet group" value={form.group_name} onChange={(event) => setForm({ ...form, group_name: event.target.value })} /></label><label>Snippet content<textarea aria-label="Snippet content" rows={5} value={form.content} onChange={(event) => setForm({ ...form, content: event.target.value })} /></label><div><button className="primary-action" type="button" onClick={() => void saveSnippet()}>Save snippet</button><button type="button" onClick={() => { setEditing(null); setFormOpen(false); setForm({ name: "", content: "", group_name: "" }); }}>Cancel</button></div></div> : null}
        <div className="settings-snippet-list">{snippets.length === 0 ? <p className="settings-empty">No snippets yet.</p> : snippets.map((snippet) => <article className="settings-snippet-row" key={snippet.id}><div><strong>{snippet.name}</strong><span>{snippet.group_name || "Ungrouped"}</span><p>{snippet.content}</p></div><div><button type="button" aria-label={`Edit ${snippet.name}`} title="Edit" onClick={() => editSnippet(snippet)}><Pencil size={15} aria-hidden="true" /></button><button className="danger" type="button" aria-label={`Delete ${snippet.name}`} title="Delete" onClick={() => void removeSnippet(snippet)}><Trash2 size={15} aria-hidden="true" /></button></div></article>)}</div>
      </section>
      <p className="clipboard-settings-status" data-kind={statusKind} role={statusKind === "error" ? "alert" : "status"}>{status}</p>
    </main>
  );
}

function SettingToggle({ label, checked, onChange }: { label: string; checked: boolean; onChange: () => void }) { return <div className="setting-toggle"><span>{label}</span><Switch aria-label={label} checked={checked} onCheckedChange={onChange} /></div>; }
function NumberSetting({ label, value, onChange }: { label: string; value: number; onChange: (value: number) => void }) { return <label className="number-setting"><span>{label}</span><input type="number" min="1" value={value} onChange={(event) => onChange(Number(event.target.value))} /></label>; }
