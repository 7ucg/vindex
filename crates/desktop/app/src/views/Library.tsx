import { createResource, createSignal, For, onMount, Show, type JSX } from "solid-js";
import {
  library,
  deleteLibraryItem,
  renameLibraryItem,
  openLibraryItem,
  localMediaUrl,
  revealLibraryItem,
  titleUrlByName,
  importFromFolder,
  pickDownloadFolder,
  type LibraryEntry,
} from "../lib/api";
import { openPlayer } from "../lib/store";
import { getTitleUrl } from "../lib/titlemap";
import { Cover } from "./Cover";
import { t } from "../lib/i18n";

// Cover einer Bibliotheks-Gruppe: erst die gemerkte Portal-URL, sonst per
// Titelname im lokalen Index nachschlagen. Ohne Treffer bleibt die Fläche leer.
function LibCover(props: { name: string }): JSX.Element {
  const [url, setUrl] = createSignal<string | null>(getTitleUrl(props.name));
  onMount(() => {
    if (!url()) {
      titleUrlByName(props.name)
        .then((u) => u && setUrl(u))
        .catch(() => {});
    }
  });
  return (
    <Show when={url()}>
      {(u) => (
        <div class="lib-group-cover">
          <Cover url={u()} title={props.name} />
        </div>
      )}
    </Show>
  );
}

function size(bytes: number): string {
  const m = bytes / (1024 * 1024);
  return m >= 1024 ? `${(m / 1024).toFixed(2)} GB` : `${m.toFixed(0)} MB`;
}
function when(unix: number): string {
  if (!unix) return "";
  return new Date(unix * 1000).toLocaleDateString();
}

// Gruppenschlüssel aus dem Dateinamen: „Show SxxExx…" → „Show", sonst „Filme & Einzelnes".
function groupKey(name: string): string {
  const m = /^(.*?)\s+S\d+E\d+/i.exec(name);
  if (m && m[1].trim()) return m[1].trim();
  return t("library.groupMisc");
}

interface Group {
  key: string;
  items: LibraryEntry[];
}

export function Library(): JSX.Element {
  const [entries, { refetch }] = createResource(library);
  const [busy, setBusy] = createSignal<string | null>(null);
  // Inline-Umbenennen: welcher Eintrag gerade bearbeitet wird + Entwurf/Fehler.
  const [editing, setEditing] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal("");
  const [renameErr, setRenameErr] = createSignal<string | null>(null);
  const startRename = (e: LibraryEntry) => {
    setRenameErr(null);
    setDraft(e.name);
    setEditing(e.name);
  };
  const [importMsg, setImportMsg] = createSignal<string | null>(null);
  const importFolder = async () => {
    const path = await pickDownloadFolder().catch(() => null);
    if (!path) return;
    setImportMsg(t("library.importing"));
    try {
      const r = await importFromFolder(path);
      setImportMsg(
        t("library.imported", { n: r.imported }) +
          (r.skipped ? t("library.skippedSuffix", { n: r.skipped }) : ""),
      );
      await refetch();
    } catch (err) {
      setImportMsg(t("library.importError", { err: String(err) }));
    }
  };
  const commitRename = async (e: LibraryEntry) => {
    const nn = draft().trim();
    if (!nn || nn === e.name) {
      setEditing(null);
      return;
    }
    setBusy(e.name);
    try {
      await renameLibraryItem(e.name, nn);
      setEditing(null);
      setRenameErr(null);
      await refetch();
    } catch (err) {
      setRenameErr(String(err));
    } finally {
      setBusy(null);
    }
  };

  const groups = (): Group[] => {
    const all = entries() ?? [];
    const map = new Map<string, LibraryEntry[]>();
    for (const e of all) {
      const k = groupKey(e.name);
      (map.get(k) ?? map.set(k, []).get(k)!).push(e);
    }
    return [...map.entries()]
      .map(([key, items]) => ({ key, items: items.sort((a, b) => a.name.localeCompare(b.name)) }))
      .sort((a, b) => a.key.localeCompare(b.key));
  };

  // Im Fenster abspielen: die Datei läuft über den Proxy (HTTP+Range) ins
  // <video> (GStreamer demuxt MKV/TS). „Extern" öffnet mpv bzw. den System-Player.
  const play = async (e: LibraryEntry) => {
    try {
      const url = await localMediaUrl(e.name);
      // Aus „Titel SxxExx" Staffel/Folge lesen und die Portal-URL im Index
      // nachschlagen — damit AniSkip auch bei Bibliotheks-Wiedergabe greift.
      let skipRef: { url: string; season: number; episode: number } | undefined;
      const m = /^(.*?)\s+S(\d+)E(\d+)/i.exec(e.name);
      if (m && m[1].trim()) {
        const season = parseInt(m[2], 10);
        const episode = parseInt(m[3], 10);
        const valid =
          Number.isInteger(season) &&
          season >= 1 &&
          season <= 100 &&
          Number.isInteger(episode) &&
          episode >= 1 &&
          episode <= 5000;
        if (valid) {
          const titleUrl = await titleUrlByName(m[1].trim()).catch(() => null);
          if (titleUrl) skipRef = { url: titleUrl, season, episode };
        }
      }
      openPlayer({ title: e.name, url, isHls: false, skipRef });
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
      <div class="row-h" style={{ "align-items": "center", gap: "10px" }}>
        <h2>{t("library.title")}</h2>
        <Show when={entries()}>
          <span class="link">{t("library.fileCount", { n: entries()!.length })}</span>
        </Show>
        <button class="chip" style={{ "margin-left": "auto" }} onClick={() => void importFolder()}>
          {t("library.importFolder")}
        </button>
        <Show when={importMsg()}>
          <span class="sleep-label">{importMsg()}</span>
        </Show>
      </div>

      <Show when={entries.loading}>
        <div class="spinner" />
      </Show>

      <Show when={entries() && entries()!.length === 0}>
        <div class="empty">{t("library.empty")}</div>
      </Show>

      <For each={groups()}>
        {(g) => (
          <>
            <div class="row-h" style={{ "align-items": "center", gap: "10px" }}>
              <LibCover name={g.key} />
              <h2 style={{ "font-size": "15px", margin: 0 }}>{g.key}</h2>
              <span class="link">{g.items.length}</span>
            </div>
            <div class="panel">
              <For each={g.items}>
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
                      <Show when={editing() === e.name} fallback={<div class="n">{e.name}</div>}>
                        <input
                          class="cfg-input"
                          style={{ width: "100%" }}
                          value={draft()}
                          onInput={(ev) => setDraft(ev.currentTarget.value)}
                          onKeyDown={(ev) => {
                            if (ev.key === "Enter") void commitRename(e);
                            if (ev.key === "Escape") setEditing(null);
                          }}
                        />
                        <Show when={renameErr()}>
                          <div class="sub" style={{ color: "var(--err)" }}>
                            {renameErr()}
                          </div>
                        </Show>
                      </Show>
                      <div class="sub">
                        {size(e.size_bytes)}
                        <Show when={e.modified}> · {when(e.modified)}</Show>
                      </div>
                    </div>
                    <div class="act">
                      <button class="chip" onClick={() => void play(e)}>
                        {t("common.play")}
                      </button>
                      <button class="chip" onClick={() => playExternal(e)}>
                        {t("library.external")}
                      </button>
                      <button
                        class="chip"
                        onClick={() => void revealLibraryItem(e.name).catch(() => {})}
                      >
                        {t("library.folder")}
                      </button>
                      <Show
                        when={editing() === e.name}
                        fallback={
                          <button class="chip" onClick={() => startRename(e)}>
                            {t("common.rename")}
                          </button>
                        }
                      >
                        <button
                          class="chip"
                          disabled={busy() === e.name}
                          onClick={() => void commitRename(e)}
                        >
                          {busy() === e.name ? "…" : t("common.save")}
                        </button>
                        <button
                          class="chip"
                          onClick={() => {
                            setEditing(null);
                            setRenameErr(null);
                          }}
                        >
                          {t("common.cancel")}
                        </button>
                      </Show>
                      <button class="chip" disabled={busy() === e.name} onClick={() => remove(e)}>
                        {busy() === e.name ? "…" : t("common.delete")}
                      </button>
                    </div>
                  </div>
                )}
              </For>
            </div>
          </>
        )}
      </For>

      <p class="note">{t("library.note")}</p>
    </div>
  );
}
