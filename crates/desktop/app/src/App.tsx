import { Switch, Match, createSignal, onMount, onCleanup, type JSX } from "solid-js";
import {
  listen,
  listDownloads,
  onDownloadUpdate,
  setTaskbarProgress,
  checkUpdate,
  installUpdate,
  type JobView,
  type UpdateInfo,
} from "./lib/api";
import { Sidebar } from "./views/Sidebar";
import { Home } from "./views/Home";
import { Settings } from "./views/Settings";
import { Show } from "solid-js";
import { PortalView } from "./views/PortalView";
import { Detail } from "./views/Detail";
import { Downloads } from "./views/Downloads";
import { Library } from "./views/Library";
import { Player } from "./views/Player";
import { CommandPalette } from "./views/CommandPalette";
import { view, playback, openTitle } from "./lib/store";
import { t, initLangFromSystem } from "./lib/i18n";

// Erlaubte Portal-Hosts für Deep-Links (spiegelt PORTAL_HOSTS in lib/backup.ts —
// bewusst lokal dupliziert, damit nichts Fremdes als Fetch-Ziel reinkommt).
const PORTAL_HOSTS = new Set([
  "aniworld.to",
  "www.aniworld.to",
  "serienstream.to",
  "www.serienstream.to",
  "s.to",
  "www.s.to",
  "filmo.to",
  "www.filmo.to",
]);

// vindex://open/<portal-url> bzw. vindex://open?url=<encoded> → Detailansicht öffnen.
// Nutzlast kommt als string[] (Prozess-Args) oder String; robust parsen, ungültiges ignorieren.
function handleDeepLink(payload: unknown): void {
  const args = Array.isArray(payload) ? payload : typeof payload === "string" ? [payload] : [];
  const token = args.find((a): a is string => typeof a === "string" && /^vindex:\/\//i.test(a));
  if (!token) return;
  if (!/^vindex:\/\/open(\/|\?|$)/i.test(token)) {
    console.warn("vindex: unbekanntes Deep-Link-Schema:", token);
    return;
  }
  // Portal-URL ziehen: erst ?url=<encoded>, sonst Pfad-Form /open/<url> (new URL geht hier
  // nicht, da host dann "open" wäre — darum Regex + decodeURIComponent).
  let portalUrl: string | null = null;
  try {
    portalUrl = new URL(token).searchParams.get("url");
  } catch {
    /* kein gültiges URL-Objekt — Pfad-Form unten */
  }
  if (!portalUrl) {
    const m = token.match(/^vindex:\/\/open\/(.+)$/i);
    if (m) {
      try {
        portalUrl = decodeURIComponent(m[1]);
      } catch {
        portalUrl = m[1];
      }
    }
  }
  if (!portalUrl) {
    console.warn("vindex: keine Portal-URL im Deep-Link:", token);
    return;
  }
  let parsed: URL;
  try {
    parsed = new URL(portalUrl);
  } catch {
    console.warn("vindex: ungültige Portal-URL im Deep-Link:", portalUrl);
    return;
  }
  if ((parsed.protocol !== "http:" && parsed.protocol !== "https:") || !PORTAL_HOSTS.has(parsed.hostname)) {
    console.warn("vindex: Deep-Link-Host nicht erlaubt:", portalUrl);
    return;
  }
  // Anzeigetitel aus dem letzten Pfadsegment (Bindestriche/Unterstriche → Leerzeichen).
  const seg = parsed.pathname.split("/").filter(Boolean).pop();
  const titel = seg ? seg.replace(/[-_]+/g, " ").trim() || parsed.hostname : parsed.hostname;
  openTitle(titel, portalUrl);
}

export default function App(): JSX.Element {
  // Gesamt-Download-Fortschritt aufs Taskleisten-Icon spiegeln — global, damit es
  // auch läuft, wenn der Downloads-Tab gerade nicht offen ist.
  onMount(() => {
    const jobs = new Map<number, JobView>();
    const DONE = ["done", "error", "cancelled"];
    let unlisten: (() => void) | undefined;
    const apply = () => {
      const active = [...jobs.values()].filter((j) => !DONE.includes(j.status));
      if (active.length === 0) {
        void setTaskbarProgress(null).catch(() => {});
        return;
      }
      let d = 0;
      let t = 0;
      for (const j of active)
        if (j.total > 0) {
          d += j.done;
          t += j.total;
        }
      const pct = t > 0 ? Math.min(100, Math.round((d * 100) / t)) : 0;
      void setTaskbarProgress(pct).catch(() => {});
    };
    listDownloads()
      .then((list) => {
        for (const j of list) jobs.set(j.id, j);
        apply();
      })
      .catch(() => {});
    onDownloadUpdate((j) => {
      jobs.set(j.id, j);
      apply();
    })
      .then((u) => (unlisten = u))
      .catch(() => {});
    onCleanup(() => unlisten?.());
  });

  // Beim Start still auf ein Update aus den GitHub-Releases prüfen.
  const [update, setUpdate] = createSignal<UpdateInfo | null>(null);
  const [installing, setInstalling] = createSignal(false);
  const [showNotes, setShowNotes] = createSignal(false);
  onMount(() => {
    initLangFromSystem();
    checkUpdate()
      .then((u) => u && setUpdate(u))
      .catch(() => {});
  });

  // Eingehende vindex://-Deep-Links (single-instance / on_open_url im Rust-Backend).
  onMount(() => {
    let unlisten: (() => void) | undefined;
    listen("deep-link", (e) => handleDeepLink(e.payload))
      .then((u) => (unlisten = u))
      .catch(() => {});
    onCleanup(() => unlisten?.());
  });
  const doInstall = () => {
    const u = update();
    if (!u) return;
    setInstalling(true);
    // installUpdate startet die App neu — kehrt im Erfolgsfall nicht zurück.
    void installUpdate(u.version).catch(() => setInstalling(false));
  };

  return (
    <div class="app">
      <Show when={update()}>
        {(u) => (
          <div class="update-banner">
            <span>
              {t("app.update.prefix")} <b>{u().version}</b> {t("app.update.suffix", { current: u().current })}
            </span>
            <button class="chip" disabled={installing()} onClick={doInstall}>
              {installing() ? t("app.update.installing") : t("app.update.install")}
            </button>
            <button class="chip" disabled={installing()} onClick={() => setUpdate(null)}>
              {t("app.update.later")}
            </button>
            <Show when={(u().notes ?? "").trim()}>
              {(notes) => (
                <>
                  <button class="chip" onClick={() => setShowNotes((v) => !v)}>
                    {showNotes() ? t("app.update.hideChangelog") : t("app.update.whatsNew")}
                  </button>
                  <Show when={showNotes()}>
                    <div
                      style={{
                        "flex-basis": "100%",
                        "white-space": "pre-wrap",
                        "max-height": "40vh",
                        overflow: "auto",
                        "font-size": "0.82em",
                        "line-height": "1.4",
                        opacity: "0.9",
                        "margin-top": "6px",
                      }}
                    >
                      {notes()}
                    </div>
                  </Show>
                </>
              )}
            </Show>
          </div>
        )}
      </Show>
      <Sidebar />
      <main class="main">
        <Switch fallback={<Home />}>
          <Match when={view() === "home"}>
            <Home />
          </Match>
          <Match when={view() === "settings"}>
            <Settings />
          </Match>
          <Match when={view() === "library"}>
            <Library />
          </Match>
          <Match when={view() === "downloads"}>
            <Downloads />
          </Match>
          <Match when={view() === "aniworld"}>
            <PortalView portal="aniworld" />
          </Match>
          <Match when={view() === "serienstream"}>
            <PortalView portal="serienstream" />
          </Match>
          <Match when={view() === "filmo"}>
            <PortalView portal="filmo" />
          </Match>
          <Match when={view() === "detail"}>
            <Detail />
          </Match>
        </Switch>
      </main>

      <Show when={playback()} keyed>
        {(p) => <Player playback={p} />}
      </Show>

      <CommandPalette />
    </div>
  );
}
