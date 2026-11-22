import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { resolvePlayback } from "../lib/api";
import { closePlayer, openPlayer, type Playback } from "../lib/store";

function epLabel(show: string, season: number, idx: number): string {
  const s = season === 0 ? "Filme" : `Staffel ${season}`;
  return `${show} — ${s} · ${idx}`;
}

const SLEEP_OPTIONS = [15, 30, 60];

// In-Window-Player. Natives <video controls> als Transport, hls.js (dynamisch) für
// HLS. Dazu: Tastatur-Shortcuts, automatisch nächste Folge und Sleep-Timer.
export function Player(props: { playback: Playback }): JSX.Element {
  let video: HTMLVideoElement | undefined;
  let hls: import("hls.js").default | undefined;
  const [err, setErr] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [sleepMin, setSleepMin] = createSignal<number | null>(null);
  const [stopAfter, setStopAfter] = createSignal(false);
  const [goingNext, setGoingNext] = createSignal(false);
  let sleepTimer: number | undefined;

  const q = () => props.playback.queue;
  const hasNext = () => {
    const c = q();
    return !!c && c.index >= 0 && c.index + 1 < c.episodes.length;
  };

  const next = async () => {
    const c = q();
    if (!c || c.index + 1 >= c.episodes.length) return;
    const ne = c.episodes[c.index + 1];
    setGoingNext(true);
    try {
      const src = await resolvePlayback(ne.url);
      openPlayer({
        title: epLabel(c.showTitle, ne.season, ne.index_in_season),
        url: src.url,
        isHls: src.is_hls,
        queue: { ...c, index: c.index + 1 },
      });
    } catch (e) {
      setErr(`Nächste Folge: ${String(e)}`);
      setGoingNext(false);
    }
  };

  const onEnded = () => {
    if (stopAfter()) {
      closePlayer();
      return;
    }
    if (hasNext()) void next();
  };

  const setSleep = (min: number | null) => {
    setSleepMin(min);
    setStopAfter(false);
    if (sleepTimer) clearTimeout(sleepTimer);
    if (min !== null) {
      sleepTimer = window.setTimeout(() => closePlayer(), min * 60 * 1000);
    }
  };

  const keys = (e: KeyboardEvent) => {
    const v = video;
    if (!v) return;
    switch (e.key) {
      case " ":
      case "k":
        e.preventDefault();
        v.paused ? void v.play() : v.pause();
        break;
      case "ArrowLeft":
        v.currentTime -= 5;
        break;
      case "ArrowRight":
        v.currentTime += 5;
        break;
      case "j":
        v.currentTime -= 10;
        break;
      case "l":
        v.currentTime += 10;
        break;
      case "ArrowUp":
        e.preventDefault();
        v.volume = Math.min(1, v.volume + 0.1);
        break;
      case "ArrowDown":
        e.preventDefault();
        v.volume = Math.max(0, v.volume - 0.1);
        break;
      case "m":
        v.muted = !v.muted;
        break;
      case "f":
        if (document.fullscreenElement) void document.exitFullscreen();
        else void v.requestFullscreen().catch(() => {});
        break;
      case "n":
        if (hasNext()) void next();
        break;
      case "Escape":
        closePlayer();
        break;
      default:
        if (/^[0-9]$/.test(e.key) && v.duration) {
          v.currentTime = (parseInt(e.key, 10) / 10) * v.duration;
        }
    }
  };

  onMount(() => {
    const el = video;
    if (!el) return;
    const src = props.playback.url;

    el.addEventListener("playing", () => setLoading(false));
    el.addEventListener("waiting", () => setLoading(true));
    el.addEventListener("ended", onEnded);
    window.addEventListener("keydown", keys);

    void (async () => {
      if (props.playback.isHls) {
        const { default: Hls } = await import("hls.js");
        if (Hls.isSupported()) {
          hls = new Hls({ enableWorker: true, lowLatencyMode: false });
          hls.on(Hls.Events.ERROR, (_e, data) => {
            if (data.fatal) {
              setErr(`Wiedergabefehler: ${data.type} / ${data.details}`);
              setLoading(false);
            }
          });
          hls.loadSource(src);
          hls.attachMedia(el);
        } else {
          el.src = src;
        }
      } else {
        el.src = src;
      }
      el.play().catch(() => {});
    })();
  });

  onCleanup(() => {
    window.removeEventListener("keydown", keys);
    if (sleepTimer) clearTimeout(sleepTimer);
    if (hls) hls.destroy();
  });

  return (
    <div class="player-overlay">
      <div class="player-top">
        <button class="chip" onClick={closePlayer}>
          ‹ Schließen
        </button>
        <span class="player-title">{props.playback.title}</span>

        <div class="player-tools">
          <Show when={hasNext()}>
            <button class="chip" disabled={goingNext()} onClick={() => void next()}>
              {goingNext() ? "lädt …" : "Nächste ›"}
            </button>
          </Show>
          <div class="sleep">
            <span class="sleep-label">Sleep</span>
            <For each={SLEEP_OPTIONS}>
              {(m) => (
                <button
                  class="chip"
                  classList={{ on: sleepMin() === m }}
                  onClick={() => setSleep(sleepMin() === m ? null : m)}
                >
                  {m}m
                </button>
              )}
            </For>
            <Show when={hasNext()}>
              <button
                class="chip"
                classList={{ on: stopAfter() }}
                onClick={() => {
                  setStopAfter(!stopAfter());
                  setSleep(null);
                }}
                title="Nach dieser Folge stoppen"
              >
                Ende Folge
              </button>
            </Show>
          </div>
        </div>
      </div>

      <Show when={loading() && !err()}>
        <div class="spinner" style={{ position: "absolute", inset: "0", margin: "auto" }} />
      </Show>

      <Show when={err()}>
        <div class="player-err">{err()}</div>
      </Show>

      <video ref={video} class="player-video" controls autoplay playsinline />

      <div class="player-hint">
        Space Play · ←/→ 5s · J/L 10s · ↑/↓ Lautstärke · M stumm · F Vollbild · N nächste · 0–9
        springen · Esc schließen
      </div>
    </div>
  );
}
