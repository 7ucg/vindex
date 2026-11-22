import { createSignal, For, Show, type JSX } from "solid-js";
import { search, resolvePlayback, type SearchHit } from "../lib/api";
import { IconSearch, IconPlay } from "../lib/icons";
import { openTitle, openPlayer } from "../lib/store";
import { recentProgress, type Resume } from "../lib/progress";
import { watchlist } from "../lib/watchlist";

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
        <h2 style={{ margin: 0, "font-size": "20px" }}>Willkommen bei vindex</h2>
        <div class="search" style={{ "min-width": "320px" }}>
          <IconSearch />
          <input
            placeholder="Katalog durchsuchen (aniworld) …"
            value={keyword()}
            onInput={(e) => onInput(e.currentTarget.value)}
            onKeyDown={(e) => e.key === "Enter" && run()}
          />
        </div>
      </div>

      <Show when={!searched() && recent().length > 0}>
        <div class="row-h">
          <h2>Weiterschauen</h2>
        </div>
        <div class="grid">
          <For each={recent()}>
            {(it) => (
              <div class="card" title={it.url} onClick={() => resume(it.url, it.r)}>
                <div class="poster">
                  <div class="ph" />
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
                    ? `${Math.round((it.r.pos / it.r.dur) * 100)}% gesehen`
                    : "weiterschauen"}
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={!searched() && saved().length > 0}>
        <div class="row-h">
          <h2>Merkliste</h2>
        </div>
        <div class="grid">
          <For each={saved()}>
            {(it) => (
              <div class="card" title={it.url} onClick={() => openTitle(it.title, it.url)}>
                <div class="poster">
                  <div class="ph" />
                  <div class="play-ov">
                    <button class="play-btn">
                      <IconPlay />
                    </button>
                  </div>
                </div>
                <div class="t">{it.title}</div>
                <div class="s">gemerkt</div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={err()}>
        <div class="panel" style={{ padding: "14px 16px", color: "var(--err)" }}>
          Fehler: {err()}
        </div>
      </Show>

      <Show when={loading()}>
        <div class="spinner" />
      </Show>

      <Show when={!loading() && searched() && hits().length === 0}>
        <div class="empty">Keine Treffer für „{keyword()}".</div>
      </Show>

      <Show when={hits().length > 0}>
        <div class="row-h">
          <h2>Treffer</h2>
          <span class="link">{hits().length} gefunden</span>
        </div>
        <div class="grid">
          <For each={hits()}>
            {(h) => (
              <div class="card" title={h.url} onClick={() => openTitle(h.title, h.url)}>
                <div class="poster">
                  <div class="ph" />
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

      <Show when={!searched() && !loading() && recent().length === 0 && saved().length === 0}>
        <div class="empty">
          Tippe einen Titel ein, um den aniworld-Katalog zu durchsuchen. Die Portal-Tabs links
          zeigen den Katalog als Grid.
        </div>
      </Show>
    </div>
  );
}
