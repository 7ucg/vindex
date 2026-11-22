import {
  createResource,
  createSignal,
  createEffect,
  For,
  Show,
  type JSX,
} from "solid-js";
import { seasons, episodes, resolvePlayback, startDownload, type Episode } from "../lib/api";
import { selected, setView, openPlayer } from "../lib/store";

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

  const [seasonList] = createResource(() => sel.url, seasons);
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

  const load = async (e: Episode) => {
    const base =
      e.season === 0
        ? `${sanitize(sel.title)} - Film ${pad(e.index_in_season)}`
        : `${sanitize(sel.title)} S${pad(e.season)}E${pad(e.index_in_season)}`;
    const title = `${sel.title} — ${seasonLabel(e.season)} · ${e.index_in_season}`;
    try {
      await startDownload(e.url, base, title);
      setView("downloads");
    } catch (err) {
      setPlayErr(String(err));
    }
  };

  const watch = async (e: Episode) => {
    setBusy(e.url);
    setPlayErr(null);
    try {
      const src = await resolvePlayback(e.url);
      const list = eps() ?? [];
      const idx = list.findIndex((x) => x.url === e.url);
      openPlayer({
        title: `${sel.title} — ${seasonLabel(e.season)} · ${e.index_in_season}`,
        url: src.url,
        isHls: src.is_hls,
        queue: { showTitle: sel.title, episodes: list, index: idx },
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
      </div>

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
