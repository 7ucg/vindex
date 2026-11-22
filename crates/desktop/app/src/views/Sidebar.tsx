import { For, type JSX } from "solid-js";
import { Logo } from "../lib/Logo";
import {
  IconHome,
  IconLibrary,
  IconDownload,
  IconTv,
  IconFilm,
  IconSettings,
} from "../lib/icons";
import { view, setView, type View } from "../lib/store";

type Item = { id: View; label: string; icon: () => JSX.Element; accent?: string };

const primary: Item[] = [
  { id: "home", label: "Start", icon: IconHome },
  { id: "library", label: "Bibliothek", icon: IconLibrary },
  { id: "downloads", label: "Downloads", icon: IconDownload },
];

const portals: Item[] = [
  { id: "aniworld", label: "aniworld.to", icon: IconTv, accent: "var(--v2)" },
  { id: "serienstream", label: "serienstream.to", icon: IconTv, accent: "var(--v1)" },
  { id: "filmo", label: "filmo.to", icon: IconFilm, accent: "var(--play)" },
];

function NavButton(props: { item: Item }): JSX.Element {
  const { item } = props;
  return (
    <button
      class="nav"
      classList={{ active: view() === item.id }}
      onClick={() => setView(item.id)}
    >
      {item.icon()}
      {item.label}
      {item.accent && <span class="dot" style={{ background: item.accent }} />}
    </button>
  );
}

export function Sidebar(): JSX.Element {
  return (
    <aside class="side">
      <div class="brand">
        <Logo class="logo" />
        <span class="word">vindex</span>
      </div>

      <For each={primary}>{(it) => <NavButton item={it} />}</For>

      <div class="nav-h">Portale</div>
      <For each={portals}>{(it) => <NavButton item={it} />}</For>

      <div class="nav-h">System</div>
      <button
        class="nav"
        classList={{ active: view() === "settings" }}
        onClick={() => setView("settings")}
      >
        <IconSettings />
        Einstellungen
      </button>

      <div class="foot">lokal · keine Telemetrie</div>
    </aside>
  );
}
