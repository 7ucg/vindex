import { Switch, Match, type JSX } from "solid-js";
import { Sidebar } from "./views/Sidebar";
import { Home } from "./views/Home";
import { Settings } from "./views/Settings";
import { Placeholder } from "./views/Placeholder";
import { PortalView } from "./views/PortalView";
import { view } from "./lib/store";

export default function App(): JSX.Element {
  return (
    <div class="app">
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
            <Placeholder
              title="Bibliothek"
              badge="bald"
              desc="Hier erscheinen alle heruntergeladenen Titel — offline abspielbar, mit Fortschritt und Speicherverwaltung."
            />
          </Match>
          <Match when={view() === "downloads"}>
            <Placeholder
              title="Downloads"
              badge="bald"
              desc="Download-Warteschlange mit Prioritäten, Pause/Fortsetzen, Bandbreitenlimit und Verlauf."
            />
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
        </Switch>
      </main>
    </div>
  );
}
