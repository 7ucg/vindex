import { For, type JSX } from "solid-js";
import { Logo } from "../lib/Logo";
import {
  IconHome,
  IconLibrary,
  IconDownload,
  IconTv,
  IconFilm,
  IconSettings,
  IconSearch,
} from "../lib/icons";
import { view, setView, setPaletteOpen, type View } from "../lib/store";
import { incognito } from "../lib/private";
import { t } from "../lib/i18n";

// label ist entweder ein Übersetzungsschlüssel (t()) oder ein fester Portalname.
type Item = { id: View; label: string; translate?: boolean; icon: () => JSX.Element; accent?: string };

const primary: Item[] = [
  { id: "home", label: "sidebar.home", translate: true, icon: IconHome },
  { id: "library", label: "sidebar.library", translate: true, icon: IconLibrary },
  { id: "downloads", label: "sidebar.downloads", translate: true, icon: IconDownload },
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
      {item.translate ? t(item.label) : item.label}
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

      <button class="nav" onClick={() => setPaletteOpen(true)}>
        <IconSearch />
        {t("sidebar.search")}
        <span class="kbd" style={{ "margin-left": "auto" }}>
          Ctrl K
        </span>
      </button>

      <For each={primary}>{(it) => <NavButton item={it} />}</For>

      <div class="nav-h">{t("sidebar.portals")}</div>
      <For each={portals}>{(it) => <NavButton item={it} />}</For>

      <div class="nav-h">{t("sidebar.system")}</div>
      <button
        class="nav"
        classList={{ active: view() === "settings" }}
        onClick={() => setView("settings")}
      >
        <IconSettings />
        {t("sidebar.settings")}
      </button>

      <div class="foot">{incognito() ? t("sidebar.footIncognito") : t("sidebar.footLocal")}</div>
    </aside>
  );
}
