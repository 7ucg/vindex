import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { resolvePlayback, aniskip, type SkipInterval } from "../lib/api";
import {
  closePlayer,
  openPlayer,
  sleepTimer,
  setSleepTimer,
  type Playback,
} from "../lib/store";
import { saveProgress, getProgress, clearProgress } from "../lib/progress";

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
  const [stopAfter, setStopAfter] = createSignal(false);
  const [goingNext, setGoingNext] = createSignal(false);
  const [skips, setSkips] = createSignal<SkipInterval[]>([]);
  const [skipTarget, setSkipTarget] = createSignal<{ end: number; label: string } | null>(null);
  let sleepHandle: number | undefined;

  const doSkip = () => {
    const t = skipTarget();
    if (t && video) {
      video.currentTime = t.end + 0.1;
      setSkipTarget(null);
    }
  };

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
    // Durchgesehen → Fortschritt löschen (nicht mehr „weiterschauen").
    const ref = props.playback.skipRef;
    if (ref) clearProgress(ref.url);
    if (stopAfter()) {
      closePlayer();
      return;
    }
    if (hasNext()) void next();
  };

  // Position regelmäßig (gedrosselt) sichern.
  let lastSave = 0;
  const saveNow = (el: HTMLVideoElement) => {
    const ref = props.playback.skipRef;
    if (!ref || !el.duration || el.currentTime < 5) return;
    const now = Date.now();
    if (now - lastSave < 5000) return;
    lastSave = now;
    saveProgress(ref.url, {
      title: props.playback.title,
      pos: el.currentTime,
      dur: el.duration,
      ts: now,
      season: ref.season,
      episode: ref.episode,
    });
  };

  // Lokalen Timer am gespeicherten Deadline neu bewaffnen (übersteht Remount).
  const armSleep = () => {
    if (sleepHandle) clearTimeout(sleepHandle);
    const s = sleepTimer();
    if (!s) return;
    const rem = s.until - Date.now();
    if (rem <= 0) {
      closePlayer();
      return;
    }
    sleepHandle = window.setTimeout(() => closePlayer(), rem);
  };

  const chooseSleep = (min: number) => {
    setStopAfter(false);
    if (sleepTimer()?.min === min) {
      setSleepTimer(null);
    } else {
      setSleepTimer({ min, until: Date.now() + min * 60 * 1000 });
    }
    armSleep();
  };

  // „Nach dieser Folge": Toggle gewinnt, Sleep-Minuten werden verworfen.
  const toggleStopAfter = () => {
    const on = !stopAfter();
    setSleepTimer(null);
    if (sleepHandle) clearTimeout(sleepHandle);
    setStopAfter(on);
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
      case "s":
        doSkip();
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
    el.addEventListener("timeupdate", () => {
      const t = el.currentTime;
      const hit = skips().find((s) => t >= s.start && t < s.end - 0.3);
      setSkipTarget(hit ? { end: hit.end, label: hit.kind === "ed" ? "Outro" : "Intro" } : null);
      saveNow(el);
    });
    // An die zuletzt gesehene Stelle springen (wenn sinnvoll).
    el.addEventListener("loadedmetadata", () => {
      const ref = props.playback.skipRef;
      if (!ref) return;
      const saved = getProgress(ref.url);
      if (saved && saved.pos > 30 && el.duration && saved.pos < el.duration * 0.95) {
        el.currentTime = saved.pos;
      }
    });
    window.addEventListener("keydown", keys);

    // AniSkip-Zeiten holen (still, falls Jikan nicht erreichbar ist).
    const ref = props.playback.skipRef;
    if (ref) {
      aniskip(ref.url, ref.season, ref.episode)
        .then(setSkips)
        .catch(() => setSkips([]));
    }

    // Sleep-Timer nach einem Remount (Auto-Next) mit Restzeit neu bewaffnen.
    armSleep();

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
    // Nur den lokalen Timer löschen — das Deadline im Store bleibt und wird nach
    // dem Remount (nächste Folge) neu bewaffnet.
    if (sleepHandle) clearTimeout(sleepHandle);
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
                  classList={{ on: sleepTimer()?.min === m }}
                  onClick={() => chooseSleep(m)}
                >
                  {m}m
                </button>
              )}
            </For>
            <Show when={hasNext()}>
              <button
                class="chip"
                classList={{ on: stopAfter() }}
                onClick={toggleStopAfter}
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

      <Show when={skipTarget()}>
        {(t) => (
          <button class="skip-btn" onClick={doSkip}>
            {t().label} überspringen <span class="skip-key">S</span>
          </button>
        )}
      </Show>

      <div class="player-hint">
        Space Play · ←/→ 5s · J/L 10s · ↑/↓ Lautstärke · M stumm · F Vollbild · S skip · N nächste
        · 0–9 springen · Esc schließen
      </div>
    </div>
  );
}
