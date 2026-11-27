import { createResource, createSignal, For, Show, onMount, type JSX } from "solid-js";
import {
  search,
  resolvePlayback,
  library,
  localMediaUrl,
  seasons,
  episodes,
  type SearchHit,
} from "../lib/api";
import { IconSearch, IconPlay } from "../lib/icons";
import { t } from "../lib/i18n";
import { openTitle, openPlayer } from "../lib/store";
import { recentProgress, type Resume } from "../lib/progress";
import { getTitleUrl } from "../lib/titlemap";
import { watchlist } from "../lib/watchlist";
import { collections, deleteCollection } from "../lib/collections";
import { Cover } from "./Cover";

// Home: eine funktionierende Katalog-Suche über den echten Kern.
// „Weiterschauen"/Empfehlungen folgen, sobald der Watch-State im Frontend hängt.
export function Home(): JSX.Element {
  const [keyword, setKeyword] = createSignal("");
  const [hits, setHits] = createSignal<SearchHit[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);
  const [searched, setSearched] = createSignal(false);
  const [recent] = createSignal(recentProgress());
  const [saved] = createSignal(watchlist());
  // Zuletzt heruntergeladene Dateien (neueste zuerst).
  const [lib] = createResource(async () => {
    const all = await library().catch(() => []);
    return all.sort((a, b) => b.modified - a.modified).slice(0, 12);
  });

  const playFile = async (name: string, title: string) => {
    try {
      const url = await localMediaUrl(name);
      openPlayer({ title, url, isHls: false });
    } catch {
      /* nichts */
    }
  };

  const resume = async (url: string, r: Resume) => {
    setErr(null);
    try {
      const src = await resolvePlayback(url);
      openPlayer({
        title: r.title,
        url: src.url,
        isHls: src.is_hls,
        skipRef: { url, season: r.season, episode: r.episode },
      });
    } catch (e) {
      setErr(String(e));
    }
  };

  // --- „Neue Folgen": nächste ungesehene Folge je gerade geschauter Serie ---
  // Dateiname-Bereinigung wie in Detail/Library, damit getTitleUrl den gemerkten Schlüssel trifft.
  const clean = (s: string) => s.replace(/[\\/:*?"<>|]/g, "").trim();
  const sLabel = (n: number) => (n === 0 ? t("home.filme") : t("home.season", { n }));

  interface NextCard {
    epUrl: string;
    titleUrl: string;
    show: string;
    season: number;
    episode: number;
    epTitle: string;
  }
  const [nextEps, setNextEps] = createSignal<NextCard[]>([]);

  onMount(() => {
    void (async () => {
      // Serien aus Weiterschauen, nach Serientitel dedupliziert, neueste zuerst.
      const seen = new Set<string>();
      const shows: { titleUrl: string; r: Resume }[] = [];
      for (const it of recent()) {
        const key = clean(it.r.title).toLowerCase();
        if (seen.has(key)) continue;
        seen.add(key);
        const titleUrl = getTitleUrl(clean(it.r.title));
        if (!titleUrl) continue; // ohne gemerkte Serien-URL nicht auflösbar
        shows.push({ titleUrl, r: it.r });
        if (shows.length >= 8) break; // API-Last begrenzen
      }
      const cards: NextCard[] = [];
      for (const s of shows) {
        try {
          const segs = await seasons(s.titleUrl);
          const inSeason = await episodes(s.titleUrl, s.r.season);
          const idx = inSeason.findIndex((e) => e.index_in_season === s.r.episode);
          let nxt = idx >= 0 ? inSeason[idx + 1] : undefined;
          if (!nxt) {
            // Ende der Staffel: erste Folge der nächstgrößeren Staffel.
            const nextSeason = segs.filter((n) => n > s.r.season).sort((a, b) => a - b)[0];
            if (nextSeason !== undefined) {
              const list = await episodes(s.titleUrl, nextSeason);
              nxt = list[0];
            }
          }
          if (!nxt) continue;
          cards.push({
            epUrl: nxt.url,
            titleUrl: s.titleUrl,
            show: s.r.title,
            season: nxt.season,
            episode: nxt.index_in_season,
            epTitle: nxt.title,
          });
          if (cards.length >= 12) break;
        } catch {
          /* Serie still überspringen */
        }
      }
      setNextEps(cards);
    })();
  });

  const playNext = async (c: NextCard) => {
    setErr(null);
    try {
      const src = await resolvePlayback(c.epUrl);
      openPlayer({
        title: `${c.show} — ${sLabel(c.season)} · ${c.episode}`,
        url: src.url,
        isHls: src.is_hls,
        skipRef: { url: c.epUrl, season: c.season, episode: c.episode },
      });
    } catch (e) {
      setErr(String(e));
    }
  };

  let timer: number | undefined;
  const onInput = (v: string) => {
    setKeyword(v);
    clearTimeout(timer);
    if (v.trim().length < 2) {
      setHits([]);
      setSearched(false);
      return;
    }
    timer = window.setTimeout(run, 280);
  };

  const run = async () => {
    const k = keyword().trim();
    if (k.length < 2) return;
    setLoading(true);
    setErr(null);
    try {
      setHits(await search(k));
      setSearched(true);
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div class="wrap">
      <div class="topbar" style={{ position: "static", padding: "0 0 18px", "border-bottom": "0", "backdrop-filter": "none", background: "none" }}>
        <h2 style={{ margin: 0, "font-size": "20px" }}>{t("home.welcome")}</h2>
        <div class="search" style={{ "min-width": "320px" }}>
          <IconSearch />
          <input
            placeholder={t("home.searchPlaceholder")}
            value={keyword()}
            onInput={(e) => onInput(e.currentTarget.value)}
            onKeyDown={(e) => e.key === "Enter" && run()}
          />
        </div>
      </div>

      <Show when={!searched() && recent().length > 0}>
        <div class="row-h">
          <h2>{t("home.continue")}</h2>
        </div>
        <div class="grid">
          <For each={recent()}>
            {(it) => (
              <div class="card" title={it.url} onClick={() => resume(it.url, it.r)}>
                <div class="poster">
                  <Cover url={it.url} title={it.r.title} />
                  <div class="play-ov">
                    <button class="play-btn">
                      <IconPlay />
                    </button>
                  </div>
                  <div class="card-progress">
                    <i
                      style={{
                        width: `${it.r.dur ? Math.min(100, (it.r.pos / it.r.dur) * 100) : 0}%`,
                      }}
                    />
                  </div>
                </div>
                <div class="t">{it.r.title}</div>
                <div class="s">
                  {it.r.dur
                    ? t("home.pctWatched", { pct: Math.round((it.r.pos / it.r.dur) * 100) })
                    : t("home.keepWatching")}
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={!searched() && nextEps().length > 0}>
        <div class="row-h">
          <h2>{t("home.newEpisodes")}</h2>
        </div>
        <div class="grid">
          <For each={nextEps()}>
            {(c) => (
              <div class="card" title={c.epUrl} onClick={() => void playNext(c)}>
                <div class="poster">
                  <Cover url={c.titleUrl} title={c.show} />
                  <div class="play-ov">
                    <button class="play-btn">
                      <IconPlay />
                    </button>
                  </div>
                </div>
                <div class="t">{c.show}</div>
                <div class="s">{c.epTitle || `${sLabel(c.season)} · ${c.episode}`}</div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={!searched() && saved().length > 0}>
        <div class="row-h">
          <h2>{t("home.watchlist")}</h2>
        </div>
        <div class="grid">
          <For each={saved()}>
            {(it) => (
              <div class="card" title={it.url} onClick={() => openTitle(it.title, it.url)}>
                <div class="poster">
                  <Cover url={it.url} title={it.title} />
                  <div class="play-ov">
                    <button class="play-btn">
                      <IconPlay />
                    </button>
                  </div>
                </div>
                <div class="t">{it.title}</div>
                <div class="s">{t("home.saved")}</div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={!searched()}>
        <For each={collections().filter((c) => c.items.length > 0)}>
          {(c) => (
            <>
              <div class="row-h">
                <h2>{c.name}</h2>
                <button
                  class="chip"
                  title={t("home.deleteCollection")}
                  onClick={() => {
                    if (confirm(t("home.deleteCollectionConfirm", { name: c.name })))
                      deleteCollection(c.id);
                  }}
                >
                  {t("common.delete")}
                </button>
              </div>
              <div class="grid">
                <For each={c.items}>
                  {(it) => (
                    <div class="card" title={it.url} onClick={() => openTitle(it.title, it.url)}>
                      <div class="poster">
                        <Cover url={it.url} title={it.title} />
                        <div class="play-ov">
                          <button class="play-btn">
                            <IconPlay />
                          </button>
                        </div>
                      </div>
                      <div class="t">{it.title}</div>
                      <div class="s">{c.name}</div>
                    </div>
                  )}
                </For>
              </div>
            </>
          )}
        </For>
      </Show>

      <Show when={!searched() && (lib()?.length ?? 0) > 0}>
        <div class="row-h">
          <h2>{t("home.recentlyAdded")}</h2>
        </div>
        <div class="grid">
          <For each={lib()}>
            {(f) => (
              <div class="card" title={f.name} onClick={() => void playFile(f.name, f.name)}>
                <div class="poster">
                  <div class="ph" />
                  <div class="play-ov">
                    <button class="play-btn">
                      <IconPlay />
                    </button>
                  </div>
                </div>
                <div class="t">{f.name}</div>
                <div class="s">{t("home.inLibrary")}</div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={err()}>
        <div class="panel" style={{ padding: "14px 16px", color: "var(--err)" }}>
          {t("common.error")}: {err()}
        </div>
      </Show>

      <Show when={loading()}>
        <div class="spinner" />
      </Show>

      <Show when={!loading() && searched() && hits().length === 0}>
        <div class="empty">{t("home.noHits", { q: keyword() })}</div>
      </Show>

      <Show when={hits().length > 0}>
        <div class="row-h">
          <h2>{t("home.hits")}</h2>
          <span class="link">{t("home.hitsFound", { n: hits().length })}</span>
        </div>
        <div class="grid">
          <For each={hits()}>
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
                <div class="s">aniworld</div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show
        when={
          !searched() &&
          !loading() &&
          recent().length === 0 &&
          saved().length === 0 &&
          (lib()?.length ?? 0) === 0
        }
      >
        <div class="empty">{t("home.emptyHint")}</div>
      </Show>
    </div>
  );
}
