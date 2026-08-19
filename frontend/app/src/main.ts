import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

type View = "history" | "favorites" | "snippets";
type Source = "history" | "favorite" | "snippet";

type Item = {
  id: number;
  source: Source;
  title?: string;
  preview_text?: string;
  content_type: string;
  source_app?: string;
  updated_at: number;
  pinned: boolean;
  group_name?: string;
};

type Settings = {
  history_enabled: boolean;
  record_sensitive: boolean;
  store_window_titles: boolean;
  max_entries: number;
  max_total_bytes: number;
  max_item_bytes: number;
};

const app = document.querySelector<HTMLDivElement>("#app")!;
let currentView: View = "history";
let query = "";
let items: Item[] = [];
let status = "Ready";
let settings: Settings | null = null;

function escapeHtml(value: string): string {
  return value.replace(/[&<>'"]/g, (character) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    "'": "&#39;",
    '"': "&quot;",
  })[character] ?? character);
}

function formatTime(milliseconds: number): string {
  return new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" }).format(milliseconds);
}

function setStatus(message: string): void {
  status = message;
  const element = document.querySelector<HTMLElement>("[data-status]");
  if (element) element.textContent = message;
}

async function loadItems(): Promise<void> {
  try {
    items = await invoke<Item[]>("quick_insert_list", { view: currentView, query, limit: 200 });
    render();
  } catch (error) {
    items = [];
    setStatus(String(error));
    render();
  }
}

async function beginQuickInsert(): Promise<void> {
  try {
    await invoke<boolean>("quick_insert_begin_session");
    setStatus("Target captured");
  } catch (error) {
    setStatus(String(error));
  }
}

async function runAction(source: Source, id: number, action: "copy" | "insert"): Promise<void> {
  try {
    const raw = await invoke<string>("quick_insert_execute", { source, id, action });
    const outcome = JSON.parse(raw) as string;
    setStatus(outcome === "inserted" ? "Inserted" : outcome === "copied" ? "Copied" : "Clipboard staged");
    if (outcome === "inserted") window.close();
  } catch (error) {
    setStatus(String(error));
  }
}

async function toggleFavorite(item: Item): Promise<void> {
  try {
    await invoke("quick_insert_set_favorite", { source: item.source, id: item.id, pinned: !item.pinned });
    await loadItems();
    setStatus(item.pinned ? "Removed from Favorites" : "Added to Favorites");
  } catch (error) {
    setStatus(String(error));
  }
}

async function deleteItem(item: Item): Promise<void> {
  try {
    await invoke("quick_insert_delete", { source: item.source, id: item.id });
    await loadItems();
    setStatus("Deleted");
  } catch (error) {
    setStatus(String(error));
  }
}

async function loadSettings(): Promise<void> {
  settings = await invoke<Settings>("settings_get");
}

async function saveSettings(): Promise<void> {
  if (!settings) return;
  try {
    await invoke("settings_update", { settings });
    setStatus("Settings updated");
  } catch (error) {
    setStatus(String(error));
  }
}

function render(): void {
  app.innerHTML = `
    <main class="shell">
      <header class="topbar">
        <div class="brand"><span class="brand-mark">E</span><div><strong>Echo</strong><small>Recall</small></div></div>
        <div class="top-actions"><button class="icon-button" data-command="settings" aria-label="Open settings" title="Settings">&#9881;</button><button class="icon-button" data-command="close" aria-label="Hide window" title="Hide">&#215;</button></div>
      </header>
      <section class="toolbar">
        <div class="segmented" role="tablist" aria-label="Library views">
          ${(["history", "favorites", "snippets"] as View[]).map((view) => `<button role="tab" aria-selected="${view === currentView}" class="segment ${view === currentView ? "active" : ""}" data-view="${view}">${view[0].toUpperCase()}${view.slice(1)}</button>`).join("")}
        </div>
        <label class="search"><span aria-hidden="true">&#8981;</span><input data-search type="search" placeholder="Search ${currentView}" value="${escapeHtml(query)}" /></label>
      </section>
      <section class="content" data-content>${renderContent()}</section>
      <footer class="statusbar"><span data-status>${escapeHtml(status)}</span><span>${items.length} items</span></footer>
    </main>
  `;
  bindEvents();
}

function renderContent(): string {
  if (currentView === "snippets") {
    return `<div class="content-head"><div><h1>Snippets</h1><p>Reusable text with a name and group.</p></div><button class="primary" data-command="new-snippet">New snippet</button></div>${items.length ? `<div class="list">${items.map(renderItem).join("")}</div>` : renderEmpty("No snippets yet")}`;
  }
  return `<div class="content-head"><div><h1>${currentView === "history" ? "History" : "Favorites"}</h1><p>${currentView === "history" ? "Your captured clipboard history." : "Saved insert items."}</p></div>${currentView === "history" ? "" : ""}</div>${items.length ? `<div class="list">${items.map(renderItem).join("")}</div>` : renderEmpty(currentView === "history" ? "Nothing captured yet" : "No favorites yet")}`;
}

function renderEmpty(message: string): string {
  return `<div class="empty"><span class="empty-mark">E</span><strong>${message}</strong><small>Keep Echo running to capture new content.</small></div>`;
}

function renderItem(item: Item): string {
  const preview = escapeHtml(item.preview_text || item.title || "Empty content");
  const source = item.source_app ? escapeHtml(item.source_app) : item.content_type;
  const isSnippet = item.source === "snippet";
  return `<article class="item" data-id="${item.id}" data-source="${item.source}">
    <div class="item-main"><div class="item-meta"><span class="type">${escapeHtml(item.content_type)}</span><span>${source}</span><time>${formatTime(item.updated_at)}</time></div><div class="preview">${preview}</div>${item.group_name ? `<div class="group">${escapeHtml(item.group_name)}</div>` : ""}</div>
    <div class="item-actions"><button data-action="copy" title="Copy">Copy</button><button data-action="insert" title="Insert">Insert</button>${isSnippet ? `<button class="danger" data-action="delete" title="Delete">Delete</button>` : `<button class="star ${item.pinned ? "on" : ""}" data-action="favorite" aria-label="Favorite" title="Favorite">&#9733;</button>`}</div>
  </article>`;
}

function renderSettings(): void {
  if (!settings) return;
  const content = document.querySelector<HTMLElement>("[data-content]");
  if (!content) return;
  content.innerHTML = `<div class="content-head"><div><h1>Settings</h1><p>Clipboard recording and retention.</p></div><button data-command="back">Back</button></div>
    <div class="settings-grid">
      <label class="setting"><span><strong>Record clipboard history</strong><small>Keep new clipboard changes in History.</small></span><input type="checkbox" data-setting="history_enabled" ${settings.history_enabled ? "checked" : ""}></label>
      <label class="setting"><span><strong>Record sensitive content</strong><small>Include password/private source classifications.</small></span><input type="checkbox" data-setting="record_sensitive" ${settings.record_sensitive ? "checked" : ""}></label>
      <label class="setting"><span><strong>Store source window titles</strong><small>Keep verified source window titles with entries.</small></span><input type="checkbox" data-setting="store_window_titles" ${settings.store_window_titles ? "checked" : ""}></label>
      <label class="number-setting"><span>Maximum entries</span><input type="number" min="1" data-setting="max_entries" value="${settings.max_entries}"></label>
      <label class="number-setting"><span>Maximum total bytes</span><input type="number" min="1" data-setting="max_total_bytes" value="${settings.max_total_bytes}"></label>
      <label class="number-setting"><span>Maximum item bytes</span><input type="number" min="1" data-setting="max_item_bytes" value="${settings.max_item_bytes}"></label>
      <div class="setting-actions"><button class="primary" data-command="save-settings">Save settings</button><button class="danger" data-command="clear-history">Clear history</button></div>
    </div>`;
  content.querySelectorAll<HTMLInputElement>("[data-setting]").forEach((input) => {
    input.addEventListener("change", () => {
      const key = input.dataset.setting as keyof Settings;
      if (key === "history_enabled" || key === "record_sensitive" || key === "store_window_titles") {
        settings![key] = input.checked;
      } else if (key === "max_entries" || key === "max_total_bytes" || key === "max_item_bytes") {
        settings![key] = Number(input.value);
      }
    });
  });
}

function renderSnippetForm(): void {
  const content = document.querySelector<HTMLElement>("[data-content]");
  if (!content) return;
  content.innerHTML = `<div class="content-head"><div><h1>New snippet</h1><p>Save text for Quick Insert.</p></div><button data-command="back">Back</button></div><form class="snippet-form"><label>Name<input required name="name" autofocus></label><label>Group<input name="group_name"></label><label>Content<textarea required name="content" rows="8"></textarea></label><button class="primary" type="submit">Save snippet</button></form>`;
  content.querySelector("form")?.addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = new FormData(event.currentTarget as HTMLFormElement);
    try {
      await invoke("snippet_save", { name: form.get("name"), content: form.get("content"), groupName: form.get("group_name") || null, id: null });
      currentView = "snippets";
      await loadItems();
      setStatus("Snippet saved");
    } catch (error) {
      setStatus(String(error));
    }
  });
}

function bindEvents(): void {
  document.querySelectorAll<HTMLButtonElement>("[data-view]").forEach((button) => button.addEventListener("click", async () => {
    currentView = button.dataset.view as View;
    query = "";
    await loadItems();
  }));
  document.querySelector<HTMLInputElement>("[data-search]")?.addEventListener("input", async (event) => {
    query = (event.currentTarget as HTMLInputElement).value;
    await loadItems();
  });
  document.querySelectorAll<HTMLElement>("[data-action]").forEach((button) => button.addEventListener("click", async () => {
    const itemElement = button.closest<HTMLElement>("[data-id]");
    if (!itemElement) return;
    const item = items.find((candidate) => candidate.id === Number(itemElement.dataset.id) && candidate.source === itemElement.dataset.source);
    if (!item) return;
    const action = button.dataset.action;
    if (action === "favorite") await toggleFavorite(item);
    else if (action === "delete") await deleteItem(item);
    else await runAction(item.source, item.id, action as "copy" | "insert");
  }));
  document.querySelectorAll<HTMLButtonElement>("[data-command]").forEach((button) => button.addEventListener("click", async () => {
    const command = button.dataset.command;
    if (command === "settings") {
      await loadSettings();
      renderSettings();
    } else if (command === "new-snippet") renderSnippetForm();
    else if (command === "save-settings") await saveSettings();
    else if (command === "clear-history") { await invoke("history_clear"); await loadItems(); setStatus("History cleared"); }
    else if (command === "back") { await loadItems(); }
    else if (command === "close") window.close();
  }));
}

type ActivationPayload = { route: string; query?: string; request_id?: string };
const handledActivations = new Set<string>();

async function applyActivation(payload: ActivationPayload): Promise<void> {
  if (payload.request_id && handledActivations.has(payload.request_id)) return;
  if (payload.request_id) handledActivations.add(payload.request_id);
  try {
    if (payload.route === "settings") {
      await loadSettings();
      renderSettings();
    } else {
      currentView = "history";
      query = payload.query || "";
      render();
      if (payload.route === "quick_insert") await beginQuickInsert();
      await loadItems();
    }
  } finally {
    if (payload.request_id) await invoke("activation_ack", { requestId: payload.request_id }).catch(() => undefined);
  }
}

void listen<ActivationPayload>("echo-activation", async ({ payload }) => {
  await applyActivation(payload);
});

render();
void (async () => {
  try {
    const pending = await invoke<ActivationPayload | null>("activation_state");
    if (pending) {
      await applyActivation(pending);
      return;
    }
  } catch (error) {
    setStatus(String(error));
  }
  await loadItems();
})();
