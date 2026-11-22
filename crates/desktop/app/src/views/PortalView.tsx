import { createResource, createSignal, createMemo, For, Show, type JSX } from "solid-js";
import { catalog, type SearchHit } from "../lib/api";
import { IconSearch, IconPlay } from "../lib/icons";
import { openTitle, type Portal } from "../lib/store";
import { Cover } from "./Cover";

type Sort = "az" | "za";

const LABEL: Record<Portal, string> = {
  aniworld: "aniworld.to",
  serienstream: "serienstream.to",
  filmo: "filmo.to",
};

// Großer Kataloge bändigen: erst mal eine Obergrenze rendern (Virtual-List folgt).
const RENDER_CAP = 300;

export function PortalView(props: { portal: Portal }): JSX.Element {
  const [items] = createResource(() => props.portal, catalog);
  const [sort, setSort] = createSignal<Sort>("az");
  const [filter, setFilter] = createSignal("");

  // Sicherer Zugriff: eine Resource im Fehlerzustand WIRFT beim Lesen von items().
  // ready() liefert die Liste, ohne zu werfen — sonst bräche die Fehleranzeige.
  const ready = () => (items.error || items.loading ? null : items() ?? []);

  const visible = createMemo(() => {
    const all: SearchHit[] = ready() ?? [];
    const f = filter().trim().toLowerCase();
    let out = f ? all.filter((h) => h.title.toLowerCase().includes(f)) : all.slice();
    out.sort((a, b) =>
      sort() === "az" ? a.title.localeCompare(b.title) : b.title.localeCompare(a.title),
    );
    return out;
  });

  return (
    <div class="wrap">
      <div class="row-h">
        <h2>{LABEL[props.portal]}</h2>
        <Show when={ready()}>
          <span class="link">{visible().length} Titel</span>
        </Show>
      </div>

      <Show when={ready()}>
        <div class="controls" style={{ "margin-bottom": "16px" }}>
          <button class="chip" classList={{ on: sort() === "az" }} onClick={() => setSort("az")}>
            A–Z
          </button>
          <button class="chip" classList={{ on: sort() === "za" }} onClick={() => setSort("za")}>
            Z–A
          </button>
          <div class="search" style={{ "margin-left": "auto", "min-width": "220px" }}>
            <IconSearch />
            <input
              placeholder="Im Katalog filtern …"
              value={filter()}
              onInput={(e) => setFilter(e.currentTarget.value)}
            />
          </div>
        </div>
      </Show>

      <Show when={items.loading}>
        <div class="spinner" />
      </Show>

      <Show when={items.error}>
        <div class="empty">
          Katalog für {LABEL[props.portal]} ist noch nicht angebunden. Discover-Endpunkt und
          Poster für dieses Portal kommen als nächster Schritt im Kern.
        </div>
      </Show>

      <Show when={ready() && visible().length === 0}>
        <div class="empty">Keine Treffer.</div>
      </Show>

      <Show when={visible().length > 0}>
        <div class="grid">
          <For each={visible().slice(0, RENDER_CAP)}>
            {(h) => (
              <div class="card" title={h.url} onClick={() => openTitle(h.title, h.url)}>
                <div class="poster">
                  <Cover url={h.url} title={h.title} />
                  <div class="play-ov">
                    <button class="play-btn">
                      <IconPlay />
                    </button>
                  </div>
                </div>
                <div class="t">{h.title}</div>
                <div class="s">{LABEL[props.portal]}</div>
              </div>
            )}
          </For>
        </div>
        <Show when={visible().length > RENDER_CAP}>
          <p class="note">
            {RENDER_CAP} von {visible().length} gezeigt — tippe zum Filtern. Endloses Scrollen
            mit Virtual-List folgt.
          </p>
        </Show>
      </Show>
    </div>
  );
}
