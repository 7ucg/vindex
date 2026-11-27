import { createSignal, createEffect, For, Show, onMount, onCleanup, type JSX } from "solid-js";
import { IconSearch } from "../lib/icons";
import { t } from "../lib/i18n";
import { openTitle, paletteOpen, setPaletteOpen, type Portal } from "../lib/store";
import { loadAll, filterHits, type PaletteHit } from "../lib/palette";

const LABEL: Record<Portal, string> = {
  aniworld: "aniworld",
  serienstream: "serienstream",
  filmo: "filmo",
};

// Globale Suchpalette (Ctrl/Cmd-K): durchsucht alle drei Kataloge auf einmal.
export function CommandPalette(): JSX.Element {
  const open = paletteOpen;
  const [query, setQuery] = createSignal("");
  const [all, setAll] = createSignal<PaletteHit[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [active, setActive] = createSignal(0);
  let input: HTMLInputElement | undefined;

  const results = (): PaletteHit[] => filterHits(all(), query());

  const hide = () => setPaletteOpen(false);

  // Beim Öffnen: zurücksetzen, Kataloge (einmalig) laden, Eingabe fokussieren.
  createEffect(() => {
    if (!open()) return;
    setQuery("");
    setActive(0);
    if (all().length === 0 && !loading()) {
      setLoading(true);
      loadAll()
        .then(setAll)
        .finally(() => setLoading(false));
    }
    queueMicrotask(() => input?.focus());
  });

  const choose = (h: PaletteHit) => {
    hide();
    openTitle(h.title, h.url);
  };

  // Markierung in gültigem Bereich halten, wenn sich die Trefferliste ändert.
  createEffect(() => {
    const n = results().length;
    if (active() >= n) setActive(n > 0 ? n - 1 : 0);
  });

  const onGlobalKey = (e: KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && (e.key === "k" || e.key === "K")) {
      e.preventDefault();
      setPaletteOpen(!open());
    }
  };

  const onPanelKey = (e: KeyboardEvent) => {
    const r = results();
    if (e.key === "Escape") {
      e.preventDefault();
      hide();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((i) => Math.min(i + 1, Math.max(0, r.length - 1)));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const h = r[active()];
      if (h) choose(h);
    }
  };

  onMount(() => {
    window.addEventListener("keydown", onGlobalKey);
    onCleanup(() => window.removeEventListener("keydown", onGlobalKey));
  });

  return (
    <Show when={open()}>
      <div class="cmdk-backdrop" onClick={hide}>
        <div class="cmdk" onClick={(e) => e.stopPropagation()} onKeyDown={onPanelKey}>
          <div class="cmdk-input">
            <IconSearch />
            <input
              ref={input}
              placeholder={t("palette.placeholder")}
              value={query()}
              onInput={(e) => {
                setQuery(e.currentTarget.value);
                setActive(0);
              }}
            />
            <span class="cmdk-esc">Esc</span>
          </div>
          <Show when={loading()}>
            <div class="cmdk-note">{t("palette.loading")}</div>
          </Show>
          <Show when={!loading() && query().trim() !== "" && results().length === 0}>
            <div class="cmdk-note">{t("palette.noHits")}</div>
          </Show>
          <Show when={query().trim() === "" && !loading()}>
            <div class="cmdk-note">{t("palette.hint")}</div>
          </Show>
          <div class="cmdk-list">
            <For each={results()}>
              {(h, i) => (
                <button
                  class="cmdk-item"
                  classList={{ on: i() === active() }}
                  onMouseEnter={() => setActive(i())}
                  onClick={() => choose(h)}
                >
                  <span class="cmdk-title">{h.title}</span>
                  <span class="cmdk-portal">{LABEL[h.portal]}</span>
                </button>
              )}
            </For>
          </div>
        </div>
      </div>
    </Show>
  );
}
