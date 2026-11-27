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
  playExternal,
  startDownload,
  episodeBase,
  titleMeta,
  library,
  localMediaUrl,
  openExternal,
  availableLanguages,
  dataSource,
  subscribe,
  unsubscribe,
  listSubscriptions,
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
import { rememberTitleUrl } from "../lib/titlemap";
import { t } from "../lib/i18n";

const LANGUAGES = ["German Dub", "German Sub", "English Sub", "English Dub"];
const HOSTERS = ["VOE", "Vidoza", "Doodstream", "Filemoon", "Vidmoly", "SpeedFiles"];
const QUALITIES = ["2160p", "1080p", "720p", "480p", "360p"];

// Windows-Gerätenamen, die als Dateiname verboten sind.
const RESERVED_NAMES = new Set([
  "CON", "PRN", "AUX", "NUL",
  "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
  "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
]);
// Härtet einen (gescrapten) Titel zu einer Datei-Namenskomponente. MUSS exakt der
// Rust-Seite entsprechen (sanitize_title_component in src-tauri/src/lib.rs), sonst
// weichen Download-Namen und der Abo-Abgleich voneinander ab (Endlos-Re-Download).
function sanitize(s: string): string {
  const filtered = Array.from(s)
    .map((c) => (/\p{Cc}/u.test(c) ? " " : c))
    .filter((c) => !/[\\/:*?"<>|]/.test(c))
    .join("");
  // Erst auf 120 Codepoints kürzen, dann normalisieren (idempotent, FS-stabil).
  let out = Array.from(filtered).slice(0, 120).join("");
  out = out.trim().replace(/^\.+|\.+$/g, "").trim();
  if (RESERVED_NAMES.has(out.toUpperCase())) out += "_";
  if (out === "") out = "titel";
  return out;
}

function seasonLabel(n: number): string {
  return n === 0 ? t("detail.seasonFilms") : t("detail.season", { n });
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

  // Titel→URL merken (bereinigter Schlüssel wie im Dateinamen), damit die Bibliothek
  // später das Cover zeigen kann. Greift schon beim bloßen Öffnen eines Titels.
  rememberTitleUrl(sanitize(sel.title), sel.url);

  const [seasonList] = createResource(() => (isMovie ? null : sel.url), seasons);
  const [meta] = createResource(() => sel.url, titleMeta);
  const [src] = createResource(() => sel.url, dataSource);
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
  // Optionale Hoster-/Qualitäts-Vorwahl pro Download ("" = Einstellungen).
  const [hoster, setHoster] = createSignal("");
  const [quality, setQuality] = createSignal("");
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
  // Im externen Player (mpv) öffnen — mit denselben Sprach-/Hoster-/Qualitäts-Vorwahlen.
  const watchExternal = async (url: string) => {
    setBusy(url);
    setPlayErr(null);
    try {
      await playExternal(url, lang(), hoster(), quality());
    } catch (e) {
      setPlayErr(String(e));
    } finally {
      setBusy(null);
    }
  };
  const loadMovie = async () => {
    try {
      await startDownload(sel.url, sanitize(sel.title), sel.title, lang(), hoster(), quality());
      setView("downloads");
    } catch (e) {
      setPlayErr(String(e));
    }
  };

  const [bulk, setBulk] = createSignal(false);
  const [saved, setSaved] = createSignal(inWatchlist(sel.url));
  const toggleSave = () => setSaved(toggleWatchlist({ url: sel.url, title: sel.title }));

  // Serien-Abo: lädt fehlende Folgen automatisch nach (Scheduler im Kern).
  // Zustand beim Öffnen aus der Abo-Liste des Kerns prüfen.
  const [subbed, setSubbed] = createSignal(false);
  const [subBusy, setSubBusy] = createSignal(false);
  listSubscriptions()
    .then((list) => setSubbed(list.some((s) => s.url === sel.url)))
    .catch(() => {});
  const toggleSub = async () => {
    setSubBusy(true);
    const next = !subbed();
    try {
      if (next) await subscribe(sel.url, sel.title, lang(), hoster(), quality());
      else await unsubscribe(sel.url);
      setSubbed(next);
    } catch (e) {
      setPlayErr(String(e));
    } finally {
      setSubBusy(false);
    }
  };

  // Sammlungen: Menü zum Zuordnen des aktuellen Titels.
  const [collMenu, setCollMenu] = createSignal(false);
  const toggleColl = (id: string) => {
    if (inCollection(id, sel.url)) removeFromCollection(id, sel.url);
    else addToCollection(id, { url: sel.url, title: sel.title });
  };
  const newColl = () => {
    const name = prompt(t("detail.newCollectionName"));
    if (name && name.trim()) {
      const id = createCollection(name);
      addToCollection(id, { url: sel.url, title: sel.title });
    }
  };

  const enqueue = (e: Episode) => {
    // Namen baut der Kern aus Staffel/Folge (inkl. Muster) — nicht hier im Frontend.
    const title = `${sel.title} — ${seasonLabel(e.season)} · ${e.index_in_season}`;
    return startDownload(
      e.url,
      "",
      title,
      lang(),
      hoster(),
      quality(),
      e.season,
      e.index_in_season,
      e.season === 0,
    );
  };

  // Basisnamen wie der Kern sie vergibt (ggf. gemustert) — fürs Offline-Erkennen,
  // damit der Abgleich zum tatsächlichen Download-Namen passt.
  const [epBases] = createResource(
    () => eps() ?? null,
    async (list) => {
      const map = new Map<string, string>();
      await Promise.all(
        list.map(async (e) => {
          map.set(e.url, await episodeBase(sel.title, e.season, e.index_in_season, e.season === 0));
        }),
      );
      return map;
    },
  );

  // Bereits heruntergeladene Folgen erkennen (Bibliothek) → direktes Offline-Abspielen.
  const [lib] = createResource(library);
  const offlineName = (e: Episode): string | null => {
    const base = epBases()?.get(e.url);
    if (!base) return null;
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
          ‹ {t("common.back")}
        </button>
        <h2 style={{ margin: 0 }}>{sel.title}</h2>
        <Show when={src()}>
          <span
            class="src-badge inline"
            classList={{ live: src() === "live" }}
            title={src() === "db" ? t("detail.srcIndexTitle") : t("detail.srcLiveTitle")}
          >
            {src() === "db" ? t("detail.srcIndex") : t("detail.srcLive")}
          </span>
        </Show>
        <div class="coll-wrap" style={{ "margin-left": "auto" }}>
          <button class="chip" classList={{ on: collMenu() }} onClick={() => setCollMenu(!collMenu())}>
            ＋ {t("detail.collection")}
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
                ＋ {t("detail.newCollection")}
              </button>
            </div>
          </Show>
        </div>
        <button class="chip" classList={{ on: saved() }} onClick={toggleSave}>
          {saved() ? `★ ${t("detail.saved")}` : `☆ ${t("detail.save")}`}
        </button>
        <Show when={!isMovie}>
          <button
            class="chip"
            classList={{ on: subbed() }}
            disabled={subBusy()}
            title={t("detail.subTitle")}
            onClick={() => void toggleSub()}
          >
            {subbed() ? `🔔 ${t("detail.subscribed")}` : `＋ ${t("detail.subscribe")}`}
          </button>
        </Show>
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
                      <span class="meta-badge fsk">{t("detail.fsk", { n: m.fsk! })}</span>
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
                    <b>{t("detail.director")}</b> {m.directors.join(", ")}
                  </p>
                </Show>
                <Show when={m.actors.length > 0}>
                  <p class="meta-people">
                    <b>{t("detail.cast")}</b> {m.actors.join(", ")}
                  </p>
                </Show>
                <Show when={m.producers.length > 0}>
                  <p class="meta-people">
                    <b>{t("detail.production")}</b> {m.producers.join(", ")}
                  </p>
                </Show>
              </div>
            </div>
          </Show>
        )}
      </Show>

      <div class="controls" style={{ "margin-bottom": "16px" }}>
        <span class="sleep-label">{t("detail.language")}</span>
        <select
          class="cfg-input"
          style={{ "max-width": "220px" }}
          value={lang()}
          onChange={(e) => setLang(e.currentTarget.value)}
        >
          <option value="">{t("detail.langAuto")}</option>
          <For each={langOptions()}>
            {(o) => (
              <option value={o.value}>
                {o.label}
                {o.sub_only ? ` · ${t("detail.subOnly")}` : ""}
              </option>
            )}
          </For>
        </select>
        <Show when={langs.loading}>
          <span class="sleep-label">{t("detail.checkingLangs")}</span>
        </Show>
        <Show when={langs() && langs()!.length === 0}>
          <span class="sleep-label" style={{ color: "var(--err)" }}>
            {t("detail.noLang")}
          </span>
        </Show>
        <span class="sleep-label">{t("detail.hoster")}</span>
        <select
          class="cfg-input"
          style={{ "max-width": "150px" }}
          value={hoster()}
          onChange={(e) => setHoster(e.currentTarget.value)}
        >
          <option value="">{t("detail.auto")}</option>
          <For each={HOSTERS}>{(h) => <option value={h}>{h}</option>}</For>
        </select>
        <span class="sleep-label">{t("detail.maxQuality")}</span>
        <select
          class="cfg-input"
          style={{ "max-width": "120px" }}
          value={quality()}
          onChange={(e) => setQuality(e.currentTarget.value)}
        >
          <option value="">{t("detail.qualityDefault")}</option>
          <For each={QUALITIES}>{(q) => <option value={q}>{q}</option>}</For>
        </select>
      </div>

      <Show when={isMovie}>
        <div class="controls" style={{ "margin-bottom": "16px" }}>
          <button class="chip on" disabled={busy() === sel.url} onClick={watchMovie}>
            {busy() === sel.url ? t("detail.resolving") : `▶ ${t("detail.watch")}`}
          </button>
          <button
            class="chip"
            disabled={busy() === sel.url}
            onClick={() => void watchExternal(sel.url)}
          >
            {t("detail.openMpv")}
          </button>
          <button class="chip" onClick={loadMovie}>
            {t("common.download")}
          </button>
        </div>
      </Show>

      <Show when={seasonList.loading}>
        <div class="spinner" />
      </Show>

      <Show when={seasonList.error}>
        <div class="empty">
          {t("detail.seasonsError", { err: String(seasonList.error) })}
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
              {bulk() ? t("detail.enqueuing") : t("detail.loadSeason")}
            </button>
            <button class="chip" disabled={bulk()} onClick={loadAll}>
              {t("detail.loadAll")}
            </button>
          </div>
        </div>
      </Show>

      <Show when={eps.loading}>
        <div class="spinner" />
      </Show>

      <Show when={eps.error}>
        <div class="empty">{t("detail.episodesError", { err: String(eps.error) })}</div>
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
                    <Show when={offlineName(e)}> · ⤓ {t("detail.offlineTag")}</Show>
                    <Show when={epSeen(e.url)}> · ✓ {t("detail.seen")}</Show>
                    <Show when={!epSeen(e.url) && epPct(e.url) > 0}>
                      {" "}· {t("detail.pctSeen", { p: epPct(e.url) })}
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
                    {busy() === e.url ? t("detail.resolving") : epPct(e.url) > 0 && !epSeen(e.url) ? t("common.resume") : t("detail.watch")}
                  </button>
                  <button
                    class="chip"
                    disabled={busy() === e.url}
                    onClick={() => void watchExternal(e.url)}
                  >
                    mpv
                  </button>
                  <Show when={offlineName(e)}>
                    {(name) => (
                      <button class="chip" onClick={() => void playOffline(e, name())}>
                        {t("detail.offline")}
                      </button>
                    )}
                  </Show>
                  <button class="chip" onClick={() => load(e)}>
                    {t("common.download")}
                  </button>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={eps() && eps()!.length === 0 && !eps.loading}>
        <div class="empty">{t("detail.noEpisodes", { label: seasonLabel(active() ?? 0) })}</div>
      </Show>

      <Show when={playErr()}>
        <div class="panel" style={{ padding: "12px 16px", color: "var(--err)", "margin-top": "12px" }}>
          {t("detail.streamError", { err: playErr()! })}
        </div>
      </Show>

      <p class="note">{t("detail.note")}</p>
    </div>
  );
}
