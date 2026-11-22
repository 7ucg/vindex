import { Switch, Match, type JSX } from "solid-js";
import { Sidebar } from "./views/Sidebar";
import { Home } from "./views/Home";
import { Settings } from "./views/Settings";
import { Placeholder } from "./views/Placeholder";
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
              phase="Phase 5"
              desc="Hier erscheinen alle heruntergeladenen Titel — offline abspielbar, mit Fortschritt und Speicherverwaltung."
            />
          </Match>
          <Match when={view() === "downloads"}>
            <Placeholder
              title="Downloads"
              phase="Phase 3"
              desc="Download-Warteschlange mit Prioritäten, Pause/Fortsetzen, Bandbreitenlimit und Verlauf."
            />
          </Match>
          <Match when={view() === "aniworld"}>
            <Placeholder
              title="aniworld.to"
              phase="Phase 1"
              desc="Katalog als Poster-Grid: nach Beliebtheit, Name oder Neu sortiert, mit Staffeln, Folgen und Filme-Tab."
            />
          </Match>
          <Match when={view() === "serienstream"}>
            <Placeholder
              title="serienstream.to"
              phase="Phase 1"
              desc="Serien unter /serie/<slug> als Poster-Grid, mit Staffel- und Folgenauswahl."
            />
          </Match>
          <Match when={view() === "filmo"}>
            <Placeholder
              title="filmo.to"
              phase="Phase 1"
              desc="Filme als Poster-Grid, direkt abspielbar oder in beliebiger Qualität ladbar."
            />
          </Match>
        </Switch>
      </main>
    </div>
  );
}
