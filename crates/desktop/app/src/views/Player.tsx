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
  const [rate, setRate] = createSignal(1);
  const [loopA, setLoopA] = createSignal<number | null>(null);
  const [loopB, setLoopB] = createSignal<number | null>(null);
  // HLS-Tracks (Qualität/Audio/Untertitel).
  type Track = { i: number; label: string };
  const [levels, setLevels] = createSignal<Track[]>([]);
  const [curLevel, setCurLevel] = createSignal(-1);
  const [audios, setAudios] = createSignal<Track[]>([]);
  const [curAudio, setCurAudio] = createSignal(-1);
  const [subs, setSubs] = createSignal<Track[]>([]);
  const [curSub, setCurSub] = createSignal(-1);
  const [menuOpen, setMenuOpen] = createSignal(false);
  let sleepHandle: number | undefined;

  const pickLevel = (i: number) => {
    if (hls) hls.currentLevel = i;
    setCurLevel(i);
  };
  const pickAudio = (i: number) => {
    if (hls) hls.audioTrack = i;
    setCurAudio(i);
  };
  const pickSub = (i: number) => {
    if (hls) hls.subtitleTrack = i;
    setCurSub(i);
  };

  const doSkip = () => {
    const t = skipTarget();
    if (t && video) {
      video.currentTime = t.end + 0.1;
      setSkipTarget(null);
    }
  };

  const setSpeed = (r: number) => {
    const nr = Math.min(3, Math.max(0.25, Math.round(r * 100) / 100));
    setRate(nr);
    if (video) video.playbackRate = nr;
  };
  // Scrollen über dem Bild = Lautstärke (an natives <video> gebunden).
  const onWheel = (e: WheelEvent) => {
    if (!video) return;
    e.preventDefault();
    const step = e.deltaY < 0 ? 0.05 : -0.05;
    video.volume = Math.min(1, Math.max(0, video.volume + step));
    if (video.muted && step > 0) video.muted = false;
  };
  const frameStep = (dir: number) => {
    if (!video) return;
    video.pause();
    video.currentTime += dir / 30;
  };
  const screenshot = () => {
    const v = video;
    if (!v || !v.videoWidth) return;
    const c = document.createElement("canvas");
    c.width = v.videoWidth;
    c.height = v.videoHeight;
    c.getContext("2d")?.drawImage(v, 0, 0);
    const safe =
      props.playback.title
        .replace(/[^\p{L}\p{N} ._-]/gu, "_")
        .replace(/^\.+/, "")
        .slice(0, 100) || "screenshot";
    const a = document.createElement("a");
    a.download = `${safe}.png`;
    a.href = c.toDataURL("image/png");
    a.click();
  };
  const togglePip = async () => {
    const v = video as (HTMLVideoElement & { requestPictureInPicture?: () => Promise<unknown> }) | undefined;
    if (!v) return;
    try {
      if (document.pictureInPictureElement) await document.exitPictureInPicture();
      else await v.requestPictureInPicture?.();
    } catch {
      /* PiP evtl. nicht verfügbar */
    }
  };
  // A/B-Loop: erst A setzen, dann B; nochmal = löschen.
  const markLoop = () => {
    if (!video) return;
    if (loopA() === null) setLoopA(video.currentTime);
    else if (loopB() === null) setLoopB(Math.max(video.currentTime, loopA()! + 0.1));
    else {
      setLoopA(null);
      setLoopB(null);
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
        v.currentTime -= e.shiftKey ? 30 : 5;
        break;
      case "ArrowRight":
        v.currentTime += e.shiftKey ? 30 : 5;
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
      case "p":
        void togglePip();
        break;
      case "[":
        setSpeed(rate() - 0.25);
        break;
      case "]":
        setSpeed(rate() + 0.25);
        break;
      case ",":
        frameStep(-1);
        break;
      case ".":
        frameStep(1);
        break;
      case "a":
        markLoop();
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
    el.addEventListener("error", () => {
      if (!props.playback.isHls) {
        setLoading(false);
        setErr("Diese Datei spielt das Fenster nicht ab — nutze Extern (mpv / System-Player).");
      }
    });
    el.addEventListener("ended", onEnded);
    el.addEventListener("timeupdate", () => {
      const t = el.currentTime;
      const hit = skips().find((s) => t >= s.start && t < s.end - 0.3);
      setSkipTarget(hit ? { end: hit.end, label: hit.kind === "ed" ? "Outro" : "Intro" } : null);
      const a = loopA();
      const b = loopB();
      if (a !== null && b !== null && t >= b) el.currentTime = a;
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

    // Fragt die Proxy-URL direkt ab, um den echten Grund zu zeigen (Status + Anfang).
    const probe = async (u: string) => {
      try {
        const r = await fetch(u);
        const body = (await r.text()).slice(0, 200).replace(/\s+/g, " ");
        setErr(`Proxy ${r.status} ${r.statusText} · ${body}`);
      } catch (e) {
        setErr(`Proxy nicht erreichbar: ${String(e)}`);
      }
    };

    void (async () => {
      if (props.playback.isHls) {
        const { default: Hls } = await import("hls.js");
        if (Hls.isSupported()) {
          hls = new Hls({ enableWorker: true, lowLatencyMode: false });
          hls.on(Hls.Events.ERROR, (_e, data) => {
            if (!data.fatal) return;
            setLoading(false);
            setErr(`Wiedergabefehler: ${data.type} / ${data.details} — prüfe Proxy …`);
            void probe(src);
          });
          hls.on(Hls.Events.MANIFEST_PARSED, () => {
            setLevels(
              hls!.levels.map((l, i) => ({
                i,
                label: l.height ? `${l.height}p` : `${Math.round((l.bitrate || 0) / 1000)}k`,
              })),
            );
            setCurLevel(hls!.currentLevel);
          });
          hls.on(Hls.Events.LEVEL_SWITCHED, (_e, d) => setCurLevel(d.level));
          hls.on(Hls.Events.AUDIO_TRACKS_UPDATED, () => {
            setAudios(
              hls!.audioTracks.map((t, i) => ({ i, label: t.name || t.lang || `Audio ${i + 1}` })),
            );
            setCurAudio(hls!.audioTrack);
          });
          hls.on(Hls.Events.SUBTITLE_TRACKS_UPDATED, () => {
            setSubs(
              hls!.subtitleTracks.map((t, i) => ({ i, label: t.name || t.lang || `Sub ${i + 1}` })),
            );
            setCurSub(hls!.subtitleTrack);
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
          <button class="chip" onClick={() => setSpeed(rate() - 0.25)} aria-label="Langsamer">−</button>
          <span class="sleep-label">{rate().toFixed(2)}×</span>
          <button class="chip" onClick={() => setSpeed(rate() + 0.25)} aria-label="Schneller">+</button>
          <button
            class="chip"
            classList={{ on: loopA() !== null }}
            onClick={markLoop}
            title="A-B-Loop (A setzen, B setzen, löschen)"
          >
            A-B
          </button>
          <button class="chip" onClick={() => void togglePip()} title="Bild-in-Bild" aria-label="Bild-in-Bild">
            PiP
          </button>
          <button class="chip" onClick={screenshot} title="Screenshot" aria-label="Screenshot">
            Foto
          </button>
          <Show when={levels().length > 1 || audios().length > 1 || subs().length > 0}>
            <button
              class="chip"
              classList={{ on: menuOpen() }}
              onClick={() => setMenuOpen(!menuOpen())}
              title="Qualität, Audio, Untertitel"
              aria-label="Qualität, Audio, Untertitel"
            >
              ⚙
            </button>
          </Show>
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

      <Show when={menuOpen()}>
        <div class="player-menu">
          <Show when={levels().length > 1}>
            <div class="pm-sec">Qualität</div>
            <div class="pm-row">
              <button class="chip" classList={{ on: curLevel() === -1 }} onClick={() => pickLevel(-1)}>
                Auto
              </button>
              <For each={levels()}>
                {(l) => (
                  <button class="chip" classList={{ on: curLevel() === l.i }} onClick={() => pickLevel(l.i)}>
                    {l.label}
                  </button>
                )}
              </For>
            </div>
          </Show>
          <Show when={audios().length > 1}>
            <div class="pm-sec">Audio</div>
            <div class="pm-row">
              <For each={audios()}>
                {(t) => (
                  <button class="chip" classList={{ on: curAudio() === t.i }} onClick={() => pickAudio(t.i)}>
                    {t.label}
                  </button>
                )}
              </For>
            </div>
          </Show>
          <Show when={subs().length > 0}>
            <div class="pm-sec">Untertitel</div>
            <div class="pm-row">
              <button class="chip" classList={{ on: curSub() === -1 }} onClick={() => pickSub(-1)}>
                Aus
              </button>
              <For each={subs()}>
                {(t) => (
                  <button class="chip" classList={{ on: curSub() === t.i }} onClick={() => pickSub(t.i)}>
                    {t.label}
                  </button>
                )}
              </For>
            </div>
          </Show>
        </div>
      </Show>

      <Show when={loading() && !err()}>
        <div class="spinner" style={{ position: "absolute", inset: "0", margin: "auto" }} />
      </Show>

      <Show when={err()}>
        <div class="player-err">{err()}</div>
      </Show>

      <video
        ref={video}
        class="player-video"
        controls
        autoplay
        playsinline
        onWheel={onWheel}
      />

      <Show when={skipTarget()}>
        {(t) => (
          <button class="skip-btn" onClick={doSkip}>
            {t().label} überspringen <span class="skip-key">S</span>
          </button>
        )}
      </Show>

      <div class="player-hint">
        Space Play · ←/→ 5s · ⇧←/→ 30s · J/L 10s · ↑/↓ oder Scroll Lautstärke · M stumm · F
        Vollbild · P PiP · S skip · N nächste · [ ] Tempo · , . Frame · A A-B-Loop · 0–9 springen ·
        Esc schließen
      </div>
    </div>
  );
}
