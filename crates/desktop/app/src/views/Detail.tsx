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
  type Episode,
} from "../lib/api";
import { selected, setView, openPlayer } from "../lib/store";
import { inWatchlist, toggleWatchlist } from "../lib/watchlist";

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
      await startDownload(sel.url, sanitize(sel.title), sel.title);
      setView("downloads");
    } catch (e) {
      setPlayErr(String(e));
    }
  };

  const [bulk, setBulk] = createSignal(false);
  const [saved, setSaved] = createSignal(inWatchlist(sel.url));
  const toggleSave = () => setSaved(toggleWatchlist({ url: sel.url, title: sel.title }));

  const enqueue = (e: Episode) => {
    const base =
      e.season === 0
        ? `${sanitize(sel.title)} - Film ${pad(e.index_in_season)}`
        : `${sanitize(sel.title)} S${pad(e.season)}E${pad(e.index_in_season)}`;
    const title = `${sel.title} — ${seasonLabel(e.season)} · ${e.index_in_season}`;
    return startDownload(e.url, base, title);
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
        <button
          class="chip"
          classList={{ on: saved() }}
          style={{ "margin-left": "auto" }}
          onClick={toggleSave}
        >
          {saved() ? "★ Gemerkt" : "☆ Merken"}
        </button>
      </div>

      <Show when={meta()?.poster || meta()?.description}>
        <div class="detail-head">
          <Show when={meta()?.poster}>
            <img class="detail-poster" src={meta()!.poster!} alt={sel.title} loading="lazy" />
          </Show>
          <Show when={meta()?.description}>
            <p class="detail-desc">{meta()!.description}</p>
          </Show>
        </div>
      </Show>

      <div class="controls" style={{ "margin-bottom": "16px" }}>
        <span class="sleep-label">Sprache</span>
        <select
          class="cfg-input"
          style={{ "max-width": "180px" }}
          value={lang()}
          onChange={(e) => setLang(e.currentTarget.value)}
        >
          <option value="">Standard</option>
          <For each={LANGUAGES}>{(l) => <option value={l}>{l}</option>}</For>
        </select>
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
                  <div class="sub">{seasonLabel(e.season)}</div>
                </div>
                <div class="act">
                  <button
                    class="chip"
                    classList={{ on: busy() === e.url }}
                    disabled={busy() === e.url}
                    onClick={() => watch(e)}
                  >
                    {busy() === e.url ? "löst auf …" : "Ansehen"}
                  </button>
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
