import { Switch, Match, createSignal, onMount, onCleanup, type JSX } from "solid-js";
import {
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
import { view, playback } from "./lib/store";

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
  onMount(() => {
    checkUpdate()
      .then((u) => u && setUpdate(u))
      .catch(() => {});
  });
  const doInstall = () => {
    setInstalling(true);
    // installUpdate startet die App neu — kehrt im Erfolgsfall nicht zurück.
    void installUpdate().catch(() => setInstalling(false));
  };

  return (
    <div class="app">
      <Show when={update()}>
        {(u) => (
          <div class="update-banner">
            <span>
              Update <b>{u().version}</b> verfügbar (aktuell {u().current}).
            </span>
            <button class="chip" disabled={installing()} onClick={doInstall}>
              {installing() ? "Installiere …" : "Installieren & neu starten"}
            </button>
            <button class="chip" disabled={installing()} onClick={() => setUpdate(null)}>
              Später
            </button>
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
