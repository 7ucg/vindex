import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { closePlayer, type Playback } from "../lib/store";

// In-Window-Player. v1: natives <video controls> als Transport (Play/Pause, Seek,
// Lautstärke, Vollbild), hls.js für HLS — dynamisch geladen, damit es das
// Start-Bundle nicht belastet. Eigene Controls, AniSkip, Auto-Next, Sleep-Timer
// und Shortcuts kommen obendrauf.
export function Player(props: { playback: Playback }): JSX.Element {
  let video: HTMLVideoElement | undefined;
  let hls: import("hls.js").default | undefined;
  const [err, setErr] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(true);

  onMount(() => {
    const el = video;
    if (!el) return;
    const src = props.playback.url;

    el.addEventListener("playing", () => setLoading(false));
    el.addEventListener("waiting", () => setLoading(true));

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closePlayer();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));

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
          // Safari: natives HLS.
          el.src = src;
        }
      } else {
        el.src = src;
      }
      el.play().catch(() => {
        /* Autoplay evtl. blockiert — die nativen Controls greifen dann. */
      });
    })();
  });

  onCleanup(() => {
    if (hls) hls.destroy();
  });

  return (
    <div class="player-overlay">
      <div class="player-top">
        <button class="chip" onClick={closePlayer}>
          ‹ Schließen
        </button>
        <span class="player-title">{props.playback.title}</span>
      </div>

      <Show when={loading() && !err()}>
        <div class="spinner" style={{ position: "absolute", inset: "0", margin: "auto" }} />
      </Show>

      <Show when={err()}>
        <div class="player-err">{err()}</div>
      </Show>

      <video ref={video} class="player-video" controls autoplay playsinline />
    </div>
  );
}
