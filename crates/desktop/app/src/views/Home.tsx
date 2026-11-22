import { createSignal, For, Show, type JSX } from "solid-js";
import { search, type SearchHit } from "../lib/api";
import { IconSearch, IconPlay } from "../lib/icons";

// Home: in Phase 0 eine funktionierende Katalog-Suche über den echten Kern.
// „Weiterschauen"/Empfehlungen folgen, sobald Watch-State im Frontend hängt (Phase 4/5).
export function Home(): JSX.Element {
  const [keyword, setKeyword] = createSignal("");
  const [hits, setHits] = createSignal<SearchHit[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [err, setErr] = createSignal<string | null>(null);
  const [searched, setSearched] = createSignal(false);

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
              <div class="card" title={h.url}>
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

      <Show when={!searched() && !loading()}>
        <div class="empty">
          Tippe einen Titel ein, um den aniworld-Katalog zu durchsuchen. Portale, Poster-Grids
          und Weiterschauen kommen mit Phase&nbsp;1.
        </div>
      </Show>
    </div>
  );
}
