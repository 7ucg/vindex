import { createSignal, onCleanup, onMount, For, Show, type JSX } from "solid-js";
import {
  listDownloads,
  cancelDownload,
  onDownloadUpdate,
  type JobView,
} from "../lib/api";

const STATUS: Record<string, string> = {
  queued: "in Warteschlange",
  resolving: "löst auf …",
  running: "lädt",
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
  if (status === "cancelled") return "var(--muted)";
  return "var(--grad)";
}

export function Downloads(): JSX.Element {
  const [jobs, setJobs] = createSignal<JobView[]>([]);
  let unlisten: (() => void) | undefined;

  onMount(async () => {
    try {
      setJobs(await listDownloads());
    } catch {
      /* leer */
    }
    unlisten = await onDownloadUpdate((job) => {
      setJobs((prev) => {
        const i = prev.findIndex((j) => j.id === job.id);
        if (i === -1) return [...prev, job];
        const copy = prev.slice();
        copy[i] = job;
        return copy;
      });
    });
  });
  onCleanup(() => unlisten?.());

  const active = () => jobs().filter((j) => !["done", "error", "cancelled"].includes(j.status));

  return (
    <div class="wrap">
      <div class="row-h">
        <h2>Downloads</h2>
        <Show when={jobs().length > 0}>
          <span class="link">{active().length} aktiv · {jobs().length} gesamt</span>
        </Show>
      </div>

      <Show when={jobs().length === 0}>
        <div class="empty">
          Noch keine Downloads. In einer Folge auf „Laden" tippen — der Fortschritt läuft hier auf.
        </div>
      </Show>

      <Show when={jobs().length > 0}>
        <div class="panel">
          <For each={jobs()}>
            {(j) => (
              <div class="dl">
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
                    <Show when={j.error}> · {j.error}</Show>
                  </div>
                  <div class="track">
                    <i style={{ width: `${pct(j)}%`, background: barColor(j.status) }} />
                  </div>
                </div>
                <div class="act">
                  <Show when={!["done", "error", "cancelled"].includes(j.status)}>
                    <button class="chip" onClick={() => cancelDownload(j.id)}>
                      Abbrechen
                    </button>
                  </Show>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>

      <p class="note">
        Download-Engine, Remux und Naming laufen im Kern. Bandbreitenlimit, Prioritäten und
        Verlauf folgen.
      </p>
    </div>
  );
}
