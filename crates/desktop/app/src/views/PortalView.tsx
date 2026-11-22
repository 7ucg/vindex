import { createResource, createSignal, createMemo, For, Show, type JSX } from "solid-js";
import { catalog, seasons, episodes, startDownload, type SearchHit } from "../lib/api";
import { IconSearch, IconPlay } from "../lib/icons";
import { openTitle, type Portal } from "../lib/store";
import { Cover } from "./Cover";

type Sort = "az" | "za";

function sanitize(s: string): string {
  return s.replace(/[\\/:*?"<>|]/g, "").trim();
}
function pad(n: number): string {
  return n < 10 ? `0${n}` : `${n}`;
}

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
  // Mehrfachauswahl → Massen-Download (url → title).
  const [selectMode, setSelectMode] = createSignal(false);
  const [picked, setPicked] = createSignal<Map<string, string>>(new Map());
  const [bulkBusy, setBulkBusy] = createSignal(false);

  const isMovie = () => props.portal === "filmo";

  const onCard = (h: SearchHit) => {
    if (!selectMode()) {
      openTitle(h.title, h.url);
      return;
    }
    setPicked((m) => {
      const n = new Map(m);
      if (n.has(h.url)) n.delete(h.url);
      else n.set(h.url, h.title);
      return n;
    });
  };

  const enqueueSeries = async (url: string, title: string) => {
    const secs = await seasons(url).catch(() => [] as number[]);
    for (const s of secs) {
      const eps = await episodes(url, s).catch(() => []);
      for (const e of eps) {
        const base = `${sanitize(title)} S${pad(e.season)}E${pad(e.index_in_season)}`;
        await startDownload(e.url, base, `${title} — S${e.season}E${e.index_in_season}`).catch(
          () => {},
        );
      }
    }
  };

  const bulkLoad = async () => {
    setBulkBusy(true);
    try {
      for (const [url, title] of picked()) {
        if (isMovie()) {
          await startDownload(url, sanitize(title), title).catch(() => {});
        } else {
          await enqueueSeries(url, title);
        }
      }
    } finally {
      setBulkBusy(false);
      setSelectMode(false);
      setPicked(new Map());
    }
  };

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
          <div class="search" style={{ "margin-left": "auto", "min-width": "200px" }}>
            <IconSearch />
            <input
              placeholder="Im Katalog filtern …"
              value={filter()}
              onInput={(e) => setFilter(e.currentTarget.value)}
            />
          </div>
          <button
            class="chip"
            classList={{ on: selectMode() }}
            onClick={() => {
              setSelectMode(!selectMode());
              setPicked(new Map());
            }}
          >
            {selectMode() ? "Fertig" : "Auswählen"}
          </button>
          <Show when={selectMode() && picked().size > 0}>
            <button class="chip" disabled={bulkBusy()} onClick={() => void bulkLoad()}>
              {bulkBusy() ? "reiht ein …" : `${picked().size} laden`}
            </button>
          </Show>
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
              <div
                class="card"
                classList={{ picked: picked().has(h.url) }}
                title={h.url}
                onClick={() => onCard(h)}
              >
                <div class="poster">
                  <Cover url={h.url} title={h.title} />
                  <Show when={selectMode()}>
                    <div class="pick-mark">{picked().has(h.url) ? "✓" : ""}</div>
                  </Show>
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
