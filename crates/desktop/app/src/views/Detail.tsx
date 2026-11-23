import {
  createResource,
  createSignal,
  createEffect,
  For,
  Show,
  type JSX,
} from "solid-js";
import {
  seasons,
  episodes,
  resolvePlayback,
  startDownload,
  titleMeta,
  library,
  localMediaUrl,
  openExternal,
  availableLanguages,
  type Episode,
  type LangOption,
} from "../lib/api";
import { selected, setView, openPlayer } from "../lib/store";
import { inWatchlist, toggleWatchlist } from "../lib/watchlist";
import { getProgress, progressVersion } from "../lib/progress";
import {
  collections,
  createCollection,
  addToCollection,
  removeFromCollection,
  inCollection,
} from "../lib/collections";

const LANGUAGES = ["German Dub", "German Sub", "English Sub", "English Dub"];

function pad(n: number): string {
  return n < 10 ? `0${n}` : `${n}`;
}
function sanitize(s: string): string {
  return s.replace(/[\\/:*?"<>|]/g, "").trim();
}

function seasonLabel(n: number): string {
  return n === 0 ? "Filme" : `Staffel ${n}`;
}

export function Detail(): JSX.Element {
  const sel = selected();

  // Ohne Auswahl zurück zur Startseite.
  if (!sel) {
    setView("home");
    return <div class="wrap" />;
  }

  // filmo listet Filme (keine Staffeln/Folgen) — eigener „Film-Modus".
  const isMovie = sel.url.includes("/movies/");

  const [seasonList] = createResource(() => (isMovie ? null : sel.url), seasons);
  const [meta] = createResource(() => sel.url, titleMeta);
  const [active, setActive] = createSignal<number | null>(null);

  // Sobald die Staffeln da sind, die erste vorauswählen.
  createEffect(() => {
    const list = seasonList();
    if (list && list.length > 0 && active() === null) setActive(list[0]);
  });

  const [eps] = createResource(
    () => (active() === null ? null : { url: sel.url, season: active()! }),
    (arg) => episodes(arg.url, arg.season),
  );

  const [busy, setBusy] = createSignal<string | null>(null);
  const [playErr, setPlayErr] = createSignal<string | null>(null);
  // Sprach-Vorwahl für die Wiedergabe ("" = Standard aus den Einstellungen).
  const [lang, setLang] = createSignal("");
  // Tatsächlich verfügbare Sprachen des Titels (Film-URL bzw. erste Episode).
  const [langs] = createResource(
    () => (isMovie ? sel.url : (eps()?.[0]?.url ?? null)),
    availableLanguages,
  );
  const langOptions = (): LangOption[] => {
    const a = langs();
    return a && a.length > 0
      ? a
      : LANGUAGES.map((l) => ({ value: l, label: l, sub_only: false }));
  };

  // Fortschritt pro Folge (reaktiv über progressVersion): Prozent + „gesehen".
  const epPct = (url: string): number => {
    progressVersion();
    const r = getProgress(url);
    if (!r || !r.dur) return 0;
    return Math.min(100, Math.round((r.pos / r.dur) * 100));
  };
  const epSeen = (url: string): boolean => epPct(url) >= 90;

  // Film-Modus: direkt die Film-URL abspielen bzw. laden.
  const watchMovie = async () => {
    setBusy(sel.url);
    setPlayErr(null);
    try {
      const src = await resolvePlayback(sel.url, lang());
      openPlayer({ title: sel.title, url: src.url, isHls: src.is_hls });
    } catch (e) {
      setPlayErr(String(e));
    } finally {
      setBusy(null);
    }
  };
  const loadMovie = async () => {
    try {
      await startDownload(sel.url, sanitize(sel.title), sel.title, lang());
      setView("downloads");
    } catch (e) {
      setPlayErr(String(e));
    }
  };

  const [bulk, setBulk] = createSignal(false);
  const [saved, setSaved] = createSignal(inWatchlist(sel.url));
  const toggleSave = () => setSaved(toggleWatchlist({ url: sel.url, title: sel.title }));

  // Sammlungen: Menü zum Zuordnen des aktuellen Titels.
  const [collMenu, setCollMenu] = createSignal(false);
  const toggleColl = (id: string) => {
    if (inCollection(id, sel.url)) removeFromCollection(id, sel.url);
    else addToCollection(id, { url: sel.url, title: sel.title });
  };
  const newColl = () => {
    const name = prompt("Name der neuen Sammlung");
    if (name && name.trim()) {
      const id = createCollection(name);
      addToCollection(id, { url: sel.url, title: sel.title });
    }
  };

  // Dateiname-Basis (ohne Endung) — identisch zur Download-Benennung.
  const epBase = (e: Episode): string =>
    e.season === 0
      ? `${sanitize(sel.title)} - Film ${pad(e.index_in_season)}`
      : `${sanitize(sel.title)} S${pad(e.season)}E${pad(e.index_in_season)}`;

  const enqueue = (e: Episode) => {
    const title = `${sel.title} — ${seasonLabel(e.season)} · ${e.index_in_season}`;
    return startDownload(e.url, epBase(e), title, lang());
  };

  // Bereits heruntergeladene Folgen erkennen (Bibliothek) → direktes Offline-Abspielen.
  const [lib] = createResource(library);
  const offlineName = (e: Episode): string | null => {
    const base = epBase(e);
    return (lib() ?? []).find((f) => f.name.startsWith(base))?.name ?? null;
  };
  const playOffline = async (e: Episode, name: string) => {
    setBusy(e.url);
    setPlayErr(null);
    try {
      const url = await localMediaUrl(name);
      const list = eps() ?? [];
      openPlayer({
        title: `${sel.title} — ${seasonLabel(e.season)} · ${e.index_in_season}`,
        url,
        isHls: false,
        queue: { showTitle: sel.title, episodes: list, index: list.findIndex((x) => x.url === e.url) },
        skipRef: { url: e.url, season: e.season, episode: e.index_in_season },
      });
    } catch (err) {
      setPlayErr(String(err));
    } finally {
      setBusy(null);
    }
  };

  const load = async (e: Episode) => {
    try {
      await enqueue(e);
      setView("downloads");
    } catch (err) {
      setPlayErr(String(err));
    }
  };

  // Ganze Staffel laden (die bereits geladene Folgenliste).
  const loadSeason = async () => {
    const list = eps() ?? [];
    setBulk(true);
    try {
      for (const e of list) await enqueue(e);
      setView("downloads");
    } catch (err) {
      setPlayErr(String(err));
    } finally {
      setBulk(false);
    }
  };

  // Komplett laden: alle Staffeln, jede Folgenliste frisch holen.
  const loadAll = async () => {
    setBulk(true);
    try {
      for (const s of seasonList() ?? []) {
        const list = await episodes(sel.url, s);
        for (const e of list) await enqueue(e);
      }
      setView("downloads");
    } catch (err) {
      setPlayErr(String(err));
    } finally {
      setBulk(false);
    }
  };

  const watch = async (e: Episode) => {
    setBusy(e.url);
    setPlayErr(null);
    try {
      const src = await resolvePlayback(e.url, lang());
      const list = eps() ?? [];
      const idx = list.findIndex((x) => x.url === e.url);
      openPlayer({
        title: `${sel.title} — ${seasonLabel(e.season)} · ${e.index_in_season}`,
        url: src.url,
        isHls: src.is_hls,
        queue: { showTitle: sel.title, episodes: list, index: idx, lang: lang() || undefined },
        skipRef: { url: e.url, season: e.season, episode: e.index_in_season },
      });
    } catch (err) {
      setPlayErr(String(err));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div class="wrap">
      <div class="row-h" style={{ "align-items": "center" }}>
        <button class="chip" onClick={() => setView(sel.from)}>
          ‹ Zurück
        </button>
        <h2 style={{ margin: 0 }}>{sel.title}</h2>
        <div class="coll-wrap" style={{ "margin-left": "auto" }}>
          <button class="chip" classList={{ on: collMenu() }} onClick={() => setCollMenu(!collMenu())}>
            ＋ Sammlung
          </button>
          <Show when={collMenu()}>
            <div class="coll-menu">
              <For each={collections()}>
                {(c) => (
                  <button class="coll-opt" onClick={() => toggleColl(c.id)}>
                    <span class="coll-check">{inCollection(c.id, sel.url) ? "✓" : ""}</span>
                    {c.name}
                  </button>
                )}
              </For>
              <button class="coll-opt coll-new" onClick={newColl}>
                ＋ Neue Sammlung …
              </button>
            </div>
          </Show>
        </div>
        <button class="chip" classList={{ on: saved() }} onClick={toggleSave}>
          {saved() ? "★ Gemerkt" : "☆ Merken"}
        </button>
      </div>

      <Show when={meta()} keyed>
        {(m) => (
          <Show
            when={
              m.poster ||
              m.description ||
              m.year ||
              m.genres.length ||
              m.directors.length ||
              m.actors.length
            }
          >
            <div class="detail-head">
              <Show when={m.poster}>
                <img class="detail-poster" src={m.poster!} alt={sel.title} loading="lazy" />
              </Show>
              <div class="detail-info">
                <Show when={m.year || m.fsk != null || m.country || m.imdb}>
                  <div class="meta-row">
                    <Show when={m.year}>
                      <span class="meta-badge">{m.year}</span>
                    </Show>
                    <Show when={m.fsk != null}>
                      <span class="meta-badge fsk">Ab {m.fsk}</span>
                    </Show>
                    <Show when={m.country}>
                      <span class="meta-dim">{m.country}</span>
                    </Show>
                    <Show when={m.imdb}>
                      <button class="meta-badge imdb" onClick={() => void openExternal(m.imdb!)}>
                        IMDB ↗
                      </button>
                    </Show>
                  </div>
                </Show>
                <Show when={m.description}>
                  <p class="detail-desc">{m.description}</p>
                </Show>
                <Show when={m.genres.length > 0}>
                  <div class="meta-chips">
                    <For each={m.genres}>{(g) => <span class="meta-chip">{g}</span>}</For>
                  </div>
                </Show>
                <Show when={m.directors.length > 0}>
                  <p class="meta-people">
                    <b>Regie:</b> {m.directors.join(", ")}
                  </p>
                </Show>
                <Show when={m.actors.length > 0}>
                  <p class="meta-people">
                    <b>Besetzung:</b> {m.actors.join(", ")}
                  </p>
                </Show>
                <Show when={m.producers.length > 0}>
                  <p class="meta-people">
                    <b>Produktion:</b> {m.producers.join(", ")}
                  </p>
                </Show>
              </div>
            </div>
          </Show>
        )}
      </Show>

      <div class="controls" style={{ "margin-bottom": "16px" }}>
        <span class="sleep-label">Sprache</span>
        <select
          class="cfg-input"
          style={{ "max-width": "220px" }}
          value={lang()}
          onChange={(e) => setLang(e.currentTarget.value)}
        >
          <option value="">Standard (automatisch)</option>
          <For each={langOptions()}>
            {(o) => (
              <option value={o.value}>
                {o.label}
                {o.sub_only ? " · nur UT" : ""}
              </option>
            )}
          </For>
        </select>
        <Show when={langs.loading}>
          <span class="sleep-label">prüfe Sprachen …</span>
        </Show>
        <Show when={langs() && langs()!.length === 0}>
          <span class="sleep-label" style={{ color: "var(--err)" }}>
            keine Sprache gefunden
          </span>
        </Show>
      </div>

      <Show when={isMovie}>
        <div class="controls" style={{ "margin-bottom": "16px" }}>
          <button class="chip on" disabled={busy() === sel.url} onClick={watchMovie}>
            {busy() === sel.url ? "löst auf …" : "▶ Ansehen"}
          </button>
          <button class="chip" onClick={loadMovie}>
            Laden
          </button>
        </div>
      </Show>

      <Show when={seasonList.loading}>
        <div class="spinner" />
      </Show>

      <Show when={seasonList.error}>
        <div class="empty">
          Konnte Staffeln nicht laden: {String(seasonList.error)}
        </div>
      </Show>

      <Show when={seasonList() && seasonList()!.length > 0}>
        <div class="controls" style={{ "margin-bottom": "16px" }}>
          <For each={seasonList()}>
            {(s) => (
              <button
                class="chip"
                classList={{ on: active() === s }}
                onClick={() => setActive(s)}
              >
                {seasonLabel(s)}
              </button>
            )}
          </For>
          <div style={{ "margin-left": "auto", display: "flex", gap: "8px" }}>
            <button class="chip" disabled={bulk()} onClick={loadSeason}>
              {bulk() ? "reiht ein …" : "Staffel laden"}
            </button>
            <button class="chip" disabled={bulk()} onClick={loadAll}>
              Alles laden
            </button>
          </div>
        </div>
      </Show>

      <Show when={eps.loading}>
        <div class="spinner" />
      </Show>

      <Show when={eps.error}>
        <div class="empty">Konnte Episoden nicht laden: {String(eps.error)}</div>
      </Show>

      <Show when={eps() && eps()!.length > 0}>
        <div class="panel">
          <For each={eps()}>
            {(e) => (
              <div class="dl" title={e.url}>
                <div
                  class="thumb"
                  style={{
                    background: "var(--surface-2)",
                    display: "grid",
                    "place-items": "center",
                    color: "var(--muted)",
                    "font-variant-numeric": "tabular-nums",
                  }}
                >
                  {e.season === 0 ? "F" : "E"}
                  {e.index_in_season}
                </div>
                <div class="info">
                  <div class="n">{e.title || `${seasonLabel(e.season)} · ${e.index_in_season}`}</div>
                  <div class="sub">
                    {seasonLabel(e.season)}
                    <Show when={offlineName(e)}> · ⤓ offline</Show>
                    <Show when={epSeen(e.url)}> · ✓ gesehen</Show>
                    <Show when={!epSeen(e.url) && epPct(e.url) > 0}>
                      {" "}· {epPct(e.url)} % gesehen
                    </Show>
                  </div>
                  <Show when={epPct(e.url) > 0}>
                    <div class="track" style={{ "margin-top": "6px" }}>
                      <i style={{ width: `${epPct(e.url)}%`, background: "var(--grad)" }} />
                    </div>
                  </Show>
                </div>
                <div class="act">
                  <button
                    class="chip"
                    classList={{ on: busy() === e.url }}
                    disabled={busy() === e.url}
                    onClick={() => watch(e)}
                  >
                    {busy() === e.url ? "löst auf …" : epPct(e.url) > 0 && !epSeen(e.url) ? "Fortsetzen" : "Ansehen"}
                  </button>
                  <Show when={offlineName(e)}>
                    {(name) => (
                      <button class="chip" onClick={() => void playOffline(e, name())}>
                        Offline
                      </button>
                    )}
                  </Show>
                  <button class="chip" onClick={() => load(e)}>
                    Laden
                  </button>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={eps() && eps()!.length === 0 && !eps.loading}>
        <div class="empty">Keine Episoden in {seasonLabel(active() ?? 0)}.</div>
      </Show>

      <Show when={playErr()}>
        <div class="panel" style={{ padding: "12px 16px", color: "var(--err)", "margin-top": "12px" }}>
          Konnte keinen Stream finden: {playErr()}
        </div>
      </Show>

      <p class="note">
        „Ansehen" spielt den Stream direkt im Fenster (über den Referer-Proxy). „Laden"
        (Download-Auswahl) wird als nächstes verdrahtet.
      </p>
    </div>
  );
}
