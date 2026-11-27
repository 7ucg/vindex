import { createSignal, onCleanup, onMount, For, Show, type JSX } from "solid-js";
import {
  listDownloads,
  cancelDownload,
  clearFinishedDownloads,
  pauseDownload,
  resumeDownload,
  retryDownload,
  setDownloadRate,
  moveDownload,
  onDownloadUpdate,
  downloadGateStatus,
  type JobView,
  type GateStatus,
} from "../lib/api";
import { getHistory, recordHistory, clearHistory, type HistItem } from "../lib/history";

const STATUS: Record<string, string> = {
  queued: "in Warteschlange",
  resolving: "löst auf …",
  running: "lädt",
  paused: "pausiert",
  muxing: "muxt",
  done: "fertig",
  error: "Fehler",
  cancelled: "abgebrochen",
};

function mb(bytes: number): string {
  if (bytes <= 0) return "";
  const m = bytes / (1024 * 1024);
  return m >= 1024 ? `${(m / 1024).toFixed(2)} GB` : `${m.toFixed(1)} MB`;
}

function pct(j: JobView): number {
  if (j.status === "done") return 100;
  if (j.total > 0) return Math.min(100, Math.round((j.done / j.total) * 100));
  return 0;
}

function barColor(status: string): string {
  if (status === "done") return "var(--ok)";
  if (status === "error") return "var(--err)";
  if (status === "cancelled" || status === "paused") return "var(--muted)";
  return "var(--grad)";
}

export function Downloads(): JSX.Element {
  const [jobs, setJobs] = createSignal<JobView[]>([]);
  // Tempo (Bytes/s) und Rest-Zeit (s) je Job, aus Byte-Deltas zwischen Events.
  const [rate, setRate] = createSignal<Record<number, { speed: number; eta: number }>>({});
  const [hist, setHist] = createSignal<HistItem[]>(getHistory());
  // Warum hält die Queue? (Zeitfenster/Akku/Metered) — für den Hinweis-Banner.
  const [gate, setGate] = createSignal<GateStatus | null>(null);
  const prev = new Map<number, { bytes: number; ts: number }>();
  // Jede Fertig-/Fehler-Meldung nur einmal in den Verlauf schreiben.
  const logged = new Set<number>();
  let unlisten: (() => void) | undefined;
  let gateTimer: ReturnType<typeof setInterval> | undefined;
  const pollGate = () => {
    void downloadGateStatus()
      .then(setGate)
      .catch(() => {});
  };

  onMount(async () => {
    try {
      setJobs(await listDownloads());
    } catch {
      /* leer */
    }
    unlisten = await onDownloadUpdate((job) => {
      const now = Date.now();
      const p = prev.get(job.id);
      if (p && job.bytes > p.bytes && now > p.ts) {
        const speed = (job.bytes - p.bytes) / ((now - p.ts) / 1000);
        let eta = 0;
        if (job.done > 0 && job.total > 0 && speed > 0) {
          const estTotal = (job.bytes / job.done) * job.total;
          eta = Math.max(0, (estTotal - job.bytes) / speed);
        }
        setRate((r) => ({ ...r, [job.id]: { speed, eta } }));
      }
      prev.set(job.id, { bytes: job.bytes, ts: now });

      // Abschluss (fertig/Fehler) einmalig im Verlauf festhalten.
      if ((job.status === "done" || job.status === "error") && !logged.has(job.id)) {
        logged.add(job.id);
        setHist(
          recordHistory({
            title: job.title,
            bytes: job.bytes,
            ts: now,
            ok: job.status === "done",
          }),
        );
      }

      setJobs((cur) => {
        const i = cur.findIndex((j) => j.id === job.id);
        if (i === -1) return [...cur, job];
        const copy = cur.slice();
        copy[i] = job;
        return copy;
      });
    });
  });
  onMount(() => {
    pollGate();
    gateTimer = setInterval(pollGate, 15000);
  });
  onCleanup(() => {
    unlisten?.();
    if (gateTimer) clearInterval(gateTimer);
  });

  const speedOf = (id: number): string => {
    const r = rate()[id];
    if (!r || !r.speed) return "";
    const m = r.speed / (1024 * 1024);
    return m >= 1 ? `${m.toFixed(1)} MB/s` : `${(r.speed / 1024).toFixed(0)} KB/s`;
  };
  const etaOf = (id: number): string => {
    const r = rate()[id];
    if (!r || !r.eta || !isFinite(r.eta)) return "";
    const s = Math.round(r.eta);
    const mm = Math.floor(s / 60);
    const ss = s % 60;
    return mm > 0 ? `${mm}:${ss.toString().padStart(2, "0")}` : `${ss}s`;
  };
  const retry = (id: number) => {
    void retryDownload(id).catch(() => {});
  };
  const when = (ts: number): string => {
    try {
      return new Date(ts).toLocaleString();
    } catch {
      return "";
    }
  };
  const emptyHist = () => {
    clearHistory();
    setHist([]);
  };

  const finished = (st: string) => ["done", "error", "cancelled"].includes(st);
  const active = () => jobs().filter((j) => !finished(j.status));
  const fmtSpeed = (bps: number): string => {
    if (!bps) return "";
    const m = bps / (1024 * 1024);
    return m >= 1 ? `${m.toFixed(1)} MB/s` : `${(bps / 1024).toFixed(0)} KB/s`;
  };
  const totalSpeed = () => active().reduce((s, j) => s + (rate()[j.id]?.speed || 0), 0);
  // Laufende zuerst, dann Wartende (nach Queue-Position), Erledigte ans Ende.
  const ordered = () => {
    const rank = (j: JobView) => (finished(j.status) ? 2 : j.status === "queued" ? 1 : 0);
    return [...jobs()].sort((a, b) => {
      const d = rank(a) - rank(b);
      if (d !== 0) return d;
      if (a.status === "queued" && b.status === "queued")
        return (a.queue_pos ?? 0) - (b.queue_pos ?? 0);
      return 0;
    });
  };
  const refresh = async () => {
    try {
      setJobs(await listDownloads());
    } catch {
      /* leer */
    }
  };
  const moveJob = async (id: number, up: boolean) => {
    await moveDownload(id, up).catch(() => {});
    await refresh();
  };
  const setJobRate = (id: number, kibStr: string) => {
    const kib = Math.max(0, parseInt(kibStr, 10) || 0);
    void setDownloadRate(id, kib).catch(() => {});
  };
  const pausable = (st: string) => ["running", "resolving", "queued"].includes(st);
  const pauseAll = () => active().forEach((j) => pausable(j.status) && void pauseDownload(j.id));
  const resumeAll = () =>
    jobs().forEach((j) => j.status === "paused" && void resumeDownload(j.id));
  // Alle noch nicht abgeschlossenen abbrechen (stoppt auch die wartende Queue).
  const cancelAll = () =>
    jobs().forEach((j) => !finished(j.status) && void cancelDownload(j.id));
  const anyFinished = () => jobs().some((j) => finished(j.status));
  const clearDone = async () => {
    await clearFinishedDownloads().catch(() => {});
    try {
      setJobs(await listDownloads());
    } catch {
      /* leer */
    }
  };

  return (
    <div class="wrap">
      <div class="row-h">
        <h2>Downloads</h2>
        <Show when={jobs().length > 0}>
          <div
            style={{
              "margin-left": "auto",
              display: "flex",
              gap: "10px",
              "align-items": "center",
            }}
          >
            <span class="link">
              {active().length} aktiv
              <Show when={totalSpeed() > 0}> · {fmtSpeed(totalSpeed())}</Show> · {jobs().length}{" "}
              gesamt
            </span>
            <Show when={active().filter((j) => pausable(j.status)).length > 1}>
              <button class="chip" onClick={pauseAll}>
                Alle pausieren
              </button>
            </Show>
            <Show when={jobs().some((j) => j.status === "paused")}>
              <button class="chip" onClick={resumeAll}>
                Alle fortsetzen
              </button>
            </Show>
            <Show when={active().length > 0}>
              <button class="chip" onClick={cancelAll}>
                Alle abbrechen
              </button>
            </Show>
            <Show when={anyFinished()}>
              <button class="chip" onClick={() => void clearDone()}>
                Erledigte leeren
              </button>
            </Show>
          </div>
        </Show>
      </div>

      <Show when={gate()?.held_reason && jobs().some((j) => j.status === "queued")}>
        <div class="note" style={{ "margin-bottom": "10px" }}>
          Warteschlange pausiert: {gate()?.held_reason} — laufende Downloads laufen zu Ende, neue
          starten erst wieder, wenn die Bedingung erfüllt ist (in den Einstellungen unter „Zeitplan
          &amp; Abos").
        </div>
      </Show>

      <Show when={jobs().length === 0}>
        <div class="empty">
          Noch keine Downloads. In einer Folge auf „Laden" tippen — der Fortschritt läuft hier auf.
        </div>
      </Show>

      <Show when={jobs().length > 0}>
        <div class="panel">
          <For each={ordered()}>
            {(j) => (
              <div class="dl" classList={{ "dl-done": finished(j.status) }}>
                <div
                  class="thumb"
                  style={{
                    background: "var(--surface-2)",
                    display: "grid",
                    "place-items": "center",
                    color: "var(--muted)",
                  }}
                >
                  {pct(j)}%
                </div>
                <div class="info">
                  <div class="n">{j.title}</div>
                  <div class="sub">
                    {STATUS[j.status] ?? j.status}
                    <Show when={j.total > 0}>
                      {" "}· {j.done}/{j.total}
                    </Show>
                    <Show when={j.bytes > 0}> · {mb(j.bytes)}</Show>
                    <Show when={j.status === "running" && speedOf(j.id)}>
                      {" "}· {speedOf(j.id)}
                    </Show>
                    <Show when={j.status === "running" && etaOf(j.id)}>
                      {" "}· noch {etaOf(j.id)}
                    </Show>
                    <Show when={j.error}> · {j.error}</Show>
                  </div>
                  <div class="track">
                    <i style={{ width: `${pct(j)}%`, background: barColor(j.status) }} />
                  </div>
                </div>
                <div class="act">
                  <Show when={j.status === "queued"}>
                    <button
                      class="chip"
                      title="In der Warteschlange nach oben"
                      onClick={() => void moveJob(j.id, true)}
                    >
                      ↑
                    </button>
                    <button
                      class="chip"
                      title="In der Warteschlange nach unten"
                      onClick={() => void moveJob(j.id, false)}
                    >
                      ↓
                    </button>
                  </Show>
                  <Show when={!finished(j.status)}>
                    <input
                      class="cfg-input"
                      style={{ width: "92px" }}
                      type="number"
                      min="0"
                      step="256"
                      placeholder="KiB/s"
                      title="Tempolimit nur für diesen Download (KiB/s, 0 = unbegrenzt)"
                      onChange={(e) => setJobRate(j.id, e.currentTarget.value)}
                    />
                  </Show>
                  <Show when={["running", "resolving", "queued"].includes(j.status)}>
                    <button class="chip" onClick={() => pauseDownload(j.id)}>
                      Pause
                    </button>
                  </Show>
                  <Show when={j.status === "paused"}>
                    <button class="chip" onClick={() => resumeDownload(j.id)}>
                      Fortsetzen
                    </button>
                  </Show>
                  <Show when={!["done", "error", "cancelled"].includes(j.status)}>
                    <button class="chip" onClick={() => cancelDownload(j.id)}>
                      Abbrechen
                    </button>
                  </Show>
                  <Show when={["error", "cancelled"].includes(j.status)}>
                    <button class="chip" onClick={() => retry(j.id)}>
                      Erneut
                    </button>
                  </Show>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={hist().length > 0}>
        <div class="row-h" style={{ margin: "22px 2px 10px" }}>
          <h2 style={{ "font-size": "14px" }}>Verlauf</h2>
          <button class="chip" onClick={emptyHist}>
            Leeren
          </button>
        </div>
        <div class="panel">
          <For each={hist()}>
            {(h) => (
              <div class="hist">
                <span class="hist-dot" classList={{ err: !h.ok }} />
                <div class="info">
                  <div class="n">{h.title}</div>
                  <div class="sub">
                    {h.ok ? "fertig" : "Fehler"} · {when(h.ts)}
                    <Show when={h.bytes > 0}> · {mb(h.bytes)}</Show>
                  </div>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <p class="note">
        Download-Engine, Remux und Naming laufen im Kern. Der Verlauf merkt sich abgeschlossene
        Downloads lokal. Bandbreitenlimit und Prioritäten folgen.
      </p>
    </div>
  );
}
