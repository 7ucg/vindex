import { createResource, createSignal, For, Show, type JSX } from "solid-js";
import {
  library,
  deleteLibraryItem,
  openLibraryItem,
  localMediaUrl,
  revealLibraryItem,
  type LibraryEntry,
} from "../lib/api";
import { openPlayer } from "../lib/store";

function size(bytes: number): string {
  const m = bytes / (1024 * 1024);
  return m >= 1024 ? `${(m / 1024).toFixed(2)} GB` : `${m.toFixed(0)} MB`;
}
function when(unix: number): string {
  if (!unix) return "";
  return new Date(unix * 1000).toLocaleDateString();
}

export function Library(): JSX.Element {
  const [entries, { refetch }] = createResource(library);
  const [busy, setBusy] = createSignal<string | null>(null);

  // Im Fenster abspielen: die Datei läuft über den Proxy (HTTP+Range) ins
  // <video> (GStreamer demuxt MKV/TS). „Extern" öffnet mpv bzw. den System-Player.
  const play = async (e: LibraryEntry) => {
    try {
      const url = await localMediaUrl(e.name);
      openPlayer({ title: e.name, url, isHls: false });
    } catch {
      void openLibraryItem(e.name).catch(() => {});
    }
  };
  const playExternal = (e: LibraryEntry) => {
    void openLibraryItem(e.name).catch(() => {});
  };

  const remove = async (e: LibraryEntry) => {
    setBusy(e.name);
    try {
      await deleteLibraryItem(e.name);
      await refetch();
    } finally {
      setBusy(null);
    }
  };

  return (
    <div class="wrap">
      <div class="row-h">
        <h2>Bibliothek</h2>
        <Show when={entries()}>
          <span class="link">{entries()!.length} Dateien</span>
        </Show>
      </div>

      <Show when={entries.loading}>
        <div class="spinner" />
      </Show>

      <Show when={entries() && entries()!.length === 0}>
        <div class="empty">
          Noch nichts heruntergeladen. Fertige Downloads landen hier und sind offline abspielbar.
        </div>
      </Show>

      <Show when={entries() && entries()!.length > 0}>
        <div class="panel">
          <For each={entries()}>
            {(e) => (
              <div class="dl" title={e.path}>
                <div
                  class="thumb"
                  style={{
                    background: "var(--surface-2)",
                    display: "grid",
                    "place-items": "center",
                    color: "var(--muted)",
                  }}
                >
                  ▶
                </div>
                <div class="info">
                  <div class="n">{e.name}</div>
                  <div class="sub">
                    {size(e.size_bytes)}
                    <Show when={e.modified}> · {when(e.modified)}</Show>
                  </div>
                </div>
                <div class="act">
                  <button class="chip" onClick={() => void play(e)}>
                    Abspielen
                  </button>
                  <button class="chip" onClick={() => playExternal(e)}>
                    Extern
                  </button>
                  <button class="chip" onClick={() => void revealLibraryItem(e.name).catch(() => {})}>
                    Ordner
                  </button>
                  <button class="chip" disabled={busy() === e.name} onClick={() => remove(e)}>
                    {busy() === e.name ? "…" : "Löschen"}
                  </button>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <p class="note">
        „Abspielen" läuft im Fenster (über den lokalen Proxy). Spielt das WebView einen Container
        nicht ab, nutze „Extern" (mpv, sonst System-Player). Löschen entfernt die Datei vom
        Datenträger.
      </p>
    </div>
  );
}
