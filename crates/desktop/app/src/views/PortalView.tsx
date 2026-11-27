import {
  createResource,
  createSignal,
  createMemo,
  createEffect,
  onMount,
  onCleanup,
  For,
  Show,
  type JSX,
} from "solid-js";
import { discover, seasons, episodes, startDownload, library, type SearchHit } from "../lib/api";
import { t } from "../lib/i18n";
import { IconSearch, IconPlay } from "../lib/icons";
import { openTitle, type Portal } from "../lib/store";
import { Cover } from "./Cover";

type Sort = "az" | "za" | "year_desc" | "year_asc" | "newest";
type Layout = "grid" | "list";

// Kuratiertes Genre-Angebot je Portal (ASCII-Slugs — werden roh in die URL gebaut).
const GENRES: Record<Portal, { slug: string; label: string }[]> = {
  aniworld: [
    ["action", "Action"],
    ["abenteuer", "Abenteuer"],
    ["drama", "Drama"],
    ["fantasy", "Fantasy"],
    ["horror", "Horror"],
    ["thriller", "Thriller"],
    ["mystery", "Mystery"],
    ["scifi", "Sci-Fi"],
    ["romanze", "Romanze"],
    ["sport", "Sport"],
    ["ecchi", "Ecchi"],
    ["harem", "Harem"],
    ["mecha", "Mecha"],
    ["krimi", "Krimi"],
  ].map(([slug, label]) => ({ slug, label })),
  serienstream: [
    ["action", "Action"],
    ["drama", "Drama"],
    ["thriller", "Thriller"],
    ["horror", "Horror"],
    ["fantasy", "Fantasy"],
    ["mystery", "Mystery"],
    ["animation", "Animation"],
    ["abenteuer", "Abenteuer"],
    ["krimi", "Krimi"],
    ["dokumentation", "Dokumentation"],
    ["western", "Western"],
  ].map(([slug, label]) => ({ slug, label })),
  filmo: [
    ["action", "Action"],
    ["adventure", "Adventure"],
    ["animation", "Animation"],
    ["comedy", "Comedy"],
    ["crime", "Crime"],
    ["documentary", "Documentary"],
    ["drama", "Drama"],
    ["family", "Family"],
    ["fantasy", "Fantasy"],
    ["history", "History"],
    ["horror", "Horror"],
    ["music", "Music"],
    ["mystery", "Mystery"],
    ["romance", "Romance"],
    ["science-fiction", "Science-Fiction"],
    ["thriller", "Thriller"],
    ["war", "War"],
    ["western", "Western"],
  ].map(([slug, label]) => ({ slug, label })),
};

// Jahr-Filter für alle drei Portale: aniworld/serienstream live über „…/jahr/<jahr>",
// filmo offline über den Index (dort steht das Jahr je Titel).
const YEARS: number[] = Array.from({ length: new Date().getFullYear() - 1969 }, (_, i) =>
  new Date().getFullYear() - i,
);

// Sprach-Filter (über den Index): nur Titel mit einem Hoster-Link in der Sprache.
const FILTER_LANGS = ["German Dub", "German Sub", "English Sub", "English Dub"];

const LAYOUT_KEY = "vindex:portallayout";
function readLayout(): Layout {
  try {
    return localStorage.getItem(LAYOUT_KEY) === "list" ? "list" : "grid";
  } catch {
    return "grid";
  }
}

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

// Virtualisierung: nur die im Viewport sichtbaren Zeilen im DOM halten. OVERSCAN
// rendert ein paar Zeilen über/unter dem Rand vor, damit das Lazy-Cover (300px
// rootMargin in Cover.tsx) rechtzeitig zieht und beim Scrollen nichts blitzt.
const OVERSCAN = 3;
// Bis die echten Maße stehen (erster Frame), diese Zahl roh rendern und dann messen.
const FALLBACK = 60;

// Nächster scrollbarer Vorfahre (hier .main); null = Fensterscroll.
function scrollParent(node: HTMLElement | null | undefined): HTMLElement | null {
  let el = node?.parentElement ?? null;
  while (el) {
    const oy = getComputedStyle(el).overflowY;
    if (oy === "auto" || oy === "scroll") return el;
    el = el.parentElement;
  }
  return null;
}

export function PortalView(props: { portal: Portal }): JSX.Element {
  const [genre, setGenre] = createSignal("");
  const [year, setYear] = createSignal(0);
  const [flang, setFlang] = createSignal("");
  const [sort, setSort] = createSignal<Sort>("az");
  // az/za sortiert der Client (kein Refetch); year_*/newest ordnet das Backend —
  // nur die lösen ein Neuladen aus. undefined = Server behält Katalog-Reihenfolge.
  const serverSort = () => (sort() === "az" || sort() === "za" ? undefined : sort());
  const [items] = createResource(
    () => ({
      portal: props.portal,
      genre: genre(),
      year: year(),
      language: flang(),
      sort: serverSort(),
    }),
    (q) =>
      discover(q.portal, q.genre || undefined, q.year || undefined, q.language || undefined, q.sort),
  );
  const [lib] = createResource(library);
  const [onlyLib, setOnlyLib] = createSignal(false);
  // Beim Portalwechsel die Filter zurücksetzen (filmo hat z. B. kein Jahr).
  createEffect(() => {
    void props.portal;
    setGenre("");
    setYear(0);
    setFlang("");
  });
  const [layout, setLayoutSig] = createSignal<Layout>(readLayout());
  const setLayout = (l: Layout) => {
    setLayoutSig(l);
    try {
      localStorage.setItem(LAYOUT_KEY, l);
    } catch {
      /* egal */
    }
  };
  const [filter, setFilter] = createSignal("");
  // Mehrfachauswahl → Massen-Download (url → title).
  const [selectMode, setSelectMode] = createSignal(false);
  const [picked, setPicked] = createSignal<Map<string, string>>(new Map());
  const [bulkBusy, setBulkBusy] = createSignal(false);

  // --- Virtualisierung ---------------------------------------------------
  let wrapEl: HTMLDivElement | undefined; // Anker, um den Scroll-Container zu finden
  let outerEl: HTMLDivElement | undefined; // stabiler Oberrand der virtualisierten Liste
  let scrollEl: HTMLElement | null = null; // .main (oder null = Fenster)
  const [scrollTop, setScrollTop] = createSignal(0);
  const [viewportH, setViewportH] = createSignal(0);
  const [gridTop, setGridTop] = createSignal(0); // Oberkante der Liste in Scroll-Koordinaten
  const [cols, setCols] = createSignal(0); // Spalten (Liste: 1)
  const [rowH, setRowH] = createSignal(0); // Zeilenhöhe inkl. gap

  const curScrollTop = () => (scrollEl ? scrollEl.scrollTop : window.scrollY);
  const curViewportH = () => (scrollEl ? scrollEl.clientHeight : window.innerHeight);
  const scrollRectTop = () => (scrollEl ? scrollEl.getBoundingClientRect().top : 0);

  // Nur Scroll-Zustand (billig, pro Frame).
  const measureScroll = () => {
    setScrollTop(curScrollTop());
    setViewportH(curViewportH());
  };
  // Geometrie: Listenoberkante, Spaltenzahl, Zeilenhöhe — aus echten DOM-Maßen,
  // damit Dichte/Theme/Layout automatisch stimmen (keine CSS-Konstanten raten).
  const measureGeom = () => {
    if (!outerEl) return;
    setGridTop(outerEl.getBoundingClientRect().top - scrollRectTop() + curScrollTop());
    const grid = outerEl.querySelector(".grid, .list") as HTMLElement | null;
    if (!grid) return;
    const sel = layout() === "grid" ? ".card" : ".list-row";
    const item = grid.querySelector(sel) as HTMLElement | null;
    if (!item) return;
    const cs = getComputedStyle(grid);
    const colGap = parseFloat(cs.columnGap) || 0;
    const rowGap = parseFloat(cs.rowGap) || 0;
    if (layout() === "list") {
      setCols(1);
    } else {
      const iw = item.offsetWidth;
      setCols(Math.max(1, Math.round((grid.clientWidth + colGap) / (iw + colGap))));
    }
    setRowH(item.offsetHeight + rowGap);
  };

  onMount(() => {
    scrollEl = scrollParent(wrapEl);
    let rafPending = false;
    const onScroll = () => {
      if (rafPending) return;
      rafPending = true;
      requestAnimationFrame(() => {
        rafPending = false;
        measureScroll();
      });
    };
    const target: HTMLElement | Window = scrollEl ?? window;
    target.addEventListener("scroll", onScroll, { passive: true });
    const ro = new ResizeObserver(() => {
      measureScroll();
      measureGeom();
    });
    if (scrollEl) ro.observe(scrollEl);
    requestAnimationFrame(() => {
      measureScroll();
      measureGeom();
    });
    onCleanup(() => {
      target.removeEventListener("scroll", onScroll);
      ro.disconnect();
    });
  });

  // Sicherer Zugriff: eine Resource im Fehlerzustand WIRFT beim Lesen von items().
  // ready() liefert die Liste, ohne zu werfen — sonst bräche die Fehleranzeige.
  const ready = () => (items.error || items.loading ? null : items() ?? []);

  // Ein Titel gilt als „in Bibliothek", wenn eine Datei mit seinem Namen beginnt.
  const inLibrary = (title: string): boolean => {
    const base = sanitize(title);
    if (!base) return false;
    return (lib() ?? []).some((f) => f.name.startsWith(base));
  };

  // Gefilterte/sortierte Trefferliste. MUSS vor der Virtualisierung (win/windowItems)
  // stehen, die visible() sofort beim Anlegen liest — sonst Temporal-Dead-Zone.
  const visible = createMemo(() => {
    const all: SearchHit[] = ready() ?? [];
    const f = filter().trim().toLowerCase();
    let out = f ? all.filter((h) => h.title.toLowerCase().includes(f)) : all.slice();
    if (onlyLib()) out = out.filter((h) => inLibrary(h.title));
    // Nur az/za clientseitig sortieren; bei den Server-Sorts (year_*/newest) die
    // Backend-Reihenfolge unverändert lassen — nicht doppelt sortieren.
    if (sort() === "az") out.sort((a, b) => a.title.localeCompare(b.title));
    else if (sort() === "za") out.sort((a, b) => b.title.localeCompare(a.title));
    return out;
  });

  // Nach Datenwechsel / Layout-Umschaltung neu vermessen, sobald das DOM steht.
  createEffect(() => {
    visible();
    layout();
    requestAnimationFrame(() => {
      measureGeom();
      measureScroll();
    });
  });

  // Bei neuer Abfrage/Filter/Sortierung nach oben springen: sonst zeigt das
  // virtualisierte Fenster bei einer jetzt kürzeren Liste unter deren Ende (leer).
  createEffect(() => {
    filter();
    genre();
    year();
    flang();
    sort();
    onlyLib();
    void props.portal;
    if (scrollEl) scrollEl.scrollTop = 0;
    else window.scrollTo(0, 0);
    setScrollTop(0);
  });

  // Sichtbares Fenster: Start/Ende-Index + Spacer-Höhen für eine korrekte Scrollleiste.
  const win = createMemo(() => {
    const n = visible().length;
    const c = cols() || 1;
    const rh = rowH();
    if (rh <= 0 || n === 0) return { start: 0, end: Math.min(n, FALLBACK), pad: 0, tail: 0 };
    const rows = Math.ceil(n / c);
    const firstRow = Math.floor((scrollTop() - gridTop()) / rh);
    // Gegen das Listenende klemmen: sonst steht das Fenster nach dem Verkleinern der
    // Liste (Filter/Sort) jenseits der letzten Zeile und der Bildschirm bleibt leer.
    const startRow = Math.min(Math.max(0, firstRow - OVERSCAN), Math.max(0, rows - 1));
    const rowsInView = Math.ceil((viewportH() || 1) / rh);
    const endRow = Math.min(rows, startRow + rowsInView + OVERSCAN * 2);
    return {
      start: startRow * c,
      end: Math.min(n, endRow * c),
      pad: startRow * rh,
      tail: Math.max(0, (rows - endRow) * rh),
    };
  });
  const windowItems = createMemo(() => visible().slice(win().start, win().end));

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
        // Staffel/Folge mitgeben: der Kern baut den Namen (inkl. naming_template) selbst,
        // damit Bulk-, Detail- und Abo-Downloads identisch benennen (Dedup bleibt heil).
        await startDownload(
          e.url,
          base,
          `${title} — S${e.season}E${e.index_in_season}`,
          undefined,
          undefined,
          undefined,
          e.season,
          e.index_in_season,
          false,
        ).catch(() => {});
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

  return (
    <div class="wrap" ref={wrapEl}>
      <div class="row-h">
        <h2>{LABEL[props.portal]}</h2>
        <Show when={ready()}>
          <span class="link">{t("portal.titles", { n: visible().length })}</span>
        </Show>
      </div>

      <Show when={ready()}>
        <div class="controls" style={{ "margin-bottom": "16px" }}>
          <button class="chip" classList={{ on: sort() === "az" }} onClick={() => setSort("az")}>
            {t("portal.sort_az")}
          </button>
          <button class="chip" classList={{ on: sort() === "za" }} onClick={() => setSort("za")}>
            {t("portal.sort_za")}
          </button>
          {/* Nur durch Index-Daten gedeckte Sortierungen. Beliebtheit/Bewertung/Status
              fehlen als Spalten (neue Crawler-Spalten + Re-Crawl nötig) — ausser Scope. */}
          <button
            class="chip"
            classList={{ on: sort() === "year_desc" }}
            onClick={() => setSort("year_desc")}
          >
            {t("portal.sort_year_desc")}
          </button>
          <button
            class="chip"
            classList={{ on: sort() === "year_asc" }}
            onClick={() => setSort("year_asc")}
          >
            {t("portal.sort_year_asc")}
          </button>
          <button
            class="chip"
            classList={{ on: sort() === "newest" }}
            onClick={() => setSort("newest")}
          >
            {t("portal.sort_newest")}
          </button>
          <button
            class="chip"
            classList={{ on: layout() === "grid" }}
            onClick={() => setLayout("grid")}
            title={t("portal.grid")}
            aria-label={t("portal.grid")}
          >
            ▦
          </button>
          <button
            class="chip"
            classList={{ on: layout() === "list" }}
            onClick={() => setLayout("list")}
            title={t("portal.list")}
            aria-label={t("portal.list")}
          >
            ☰
          </button>
          <select
            class="cfg-input"
            style={{ "max-width": "150px" }}
            value={genre()}
            onChange={(e) => {
              setGenre(e.currentTarget.value);
              if (e.currentTarget.value) setYear(0);
            }}
          >
            <option value="">{t("portal.all_genres")}</option>
            <For each={GENRES[props.portal]}>
              {(g) => <option value={g.slug}>{g.label}</option>}
            </For>
          </select>
          <select
            class="cfg-input"
            style={{ "max-width": "110px" }}
            value={year() || ""}
            onChange={(e) => {
              const y = parseInt(e.currentTarget.value, 10) || 0;
              setYear(y);
              if (y) setGenre("");
            }}
          >
            <option value="">{t("portal.all_years")}</option>
            <For each={YEARS}>{(y) => <option value={y}>{y}</option>}</For>
          </select>
          <select
            class="cfg-input"
            style={{ "max-width": "140px" }}
            value={flang()}
            onChange={(e) => setFlang(e.currentTarget.value)}
          >
            <option value="">{t("portal.all_langs")}</option>
            <For each={FILTER_LANGS}>{(l) => <option value={l}>{l}</option>}</For>
          </select>
          <Show when={(lib() ?? []).length > 0}>
            <button
              class="chip"
              classList={{ on: onlyLib() }}
              onClick={() => setOnlyLib(!onlyLib())}
              title={t("portal.only_lib_title")}
            >
              {t("portal.only_lib")}
            </button>
          </Show>
          <div class="search" style={{ "margin-left": "auto", "min-width": "200px" }}>
            <IconSearch />
            <input
              placeholder={t("portal.filter_ph")}
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
            {selectMode() ? t("common.done") : t("portal.select")}
          </button>
          <Show when={selectMode() && picked().size > 0}>
            <button class="chip" disabled={bulkBusy()} onClick={() => void bulkLoad()}>
              {bulkBusy() ? t("portal.enqueuing") : t("portal.load_n", { n: picked().size })}
            </button>
          </Show>
        </div>
      </Show>

      <Show when={items.loading}>
        <div class="spinner" />
      </Show>

      <Show when={items.error}>
        <div class="empty">{t("portal.not_connected", { portal: LABEL[props.portal] })}</div>
      </Show>

      <Show when={ready() && visible().length === 0}>
        <div class="empty">{t("portal.no_hits")}</div>
      </Show>

      <Show when={visible().length > 0}>
        {/* Virtualisierung: Spacer oben/unten halten die Gesamthöhe, dazwischen nur
            das sichtbare Fenster. Optik/Grid-Layout unverändert. */}
        <div class="virt" ref={outerEl}>
          <div class="virt-pad" style={{ height: `${win().pad}px` }} />
          <Show
            when={layout() === "grid"}
            fallback={
              <div class="list">
                <For each={windowItems()}>
                  {(h) => (
                    <div
                      class="list-row"
                      classList={{ picked: picked().has(h.url) }}
                      title={h.url}
                      onClick={() => onCard(h)}
                    >
                      <div class="list-thumb">
                        <Cover url={h.url} title={h.title} />
                        <Show when={selectMode()}>
                          <div class="pick-mark">{picked().has(h.url) ? "✓" : ""}</div>
                        </Show>
                      </div>
                      <div class="list-title">{h.title}</div>
                      <div class="list-portal">{LABEL[props.portal]}</div>
                    </div>
                  )}
                </For>
              </div>
            }
          >
            <div class="grid">
              <For each={windowItems()}>
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
                      <Show when={h.source}>
                        <div
                          class="src-badge"
                          classList={{ live: h.source === "live" }}
                          title={
                            h.source === "db"
                              ? t("portal.src_db_title")
                              : t("portal.src_live_title")
                          }
                        >
                          {h.source === "db" ? t("portal.src_db") : t("portal.src_live")}
                        </div>
                      </Show>
                      <div class="play-ov" aria-hidden="true">
                        <button class="play-btn" tabindex="-1">
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
          </Show>
          <div class="virt-pad" style={{ height: `${win().tail}px` }} />
        </div>
      </Show>
    </div>
  );
}
