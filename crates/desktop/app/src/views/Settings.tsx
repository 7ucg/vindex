import { createResource, createSignal, For, Show, type JSX } from "solid-js";
import {
  appSettings,
  appInfo,
  autostartEnabled,
  setAutostart,
  getConfig,
  setConfig,
  type DesktopConfig,
} from "../lib/api";

import { exportState, importState } from "../lib/backup";

const LANGUAGES = ["German Dub", "German Sub", "English Sub", "English Dub"];
const QUALITIES = ["2160p", "1080p", "720p", "480p", "360p"];
import {
  applyTheme,
  getTheme,
  applyAccent,
  getAccent,
  applyDensity,
  getDensity,
  applyScale,
  getScale,
  type Theme,
  type Accent,
  type Density,
  type Scale,
} from "../lib/theme";

const THEMES: { id: Theme; label: string }[] = [
  { id: "dark", label: "Dunkel" },
  { id: "light", label: "Hell" },
  { id: "amoled", label: "AMOLED" },
];
const ACCENTS: { id: Accent; label: string }[] = [
  { id: "violett", label: "Violett" },
  { id: "cyan", label: "Cyan" },
  { id: "gruen", label: "Grün" },
  { id: "pink", label: "Pink" },
  { id: "bernstein", label: "Bernstein" },
];
const DENSITIES: { id: Density; label: string }[] = [
  { id: "normal", label: "Normal" },
  { id: "kompakt", label: "Kompakt" },
];
const SCALES: { id: Scale; label: string }[] = [
  { id: "90", label: "90 %" },
  { id: "100", label: "100 %" },
  { id: "110", label: "110 %" },
  { id: "125", label: "125 %" },
];

// Settings: zeigt die effektiven Laufzeit-Einstellungen (VINDEX_* / .env), live aus dem Kern.
export function Settings(): JSX.Element {
  const [settings] = createResource(appSettings);
  const [info] = createResource(appInfo);
  const [theme, setTheme] = createSignal<Theme>(getTheme());
  const [accent, setAccent] = createSignal<Accent>(getAccent());
  const [density, setDensity] = createSignal<Density>(getDensity());
  const [scale, setScale] = createSignal<Scale>(getScale());
  const [autostart, setAutostartSig] = createSignal(false);
  autostartEnabled()
    .then(setAutostartSig)
    .catch(() => {});
  const toggleAutostart = () => {
    const next = !autostart();
    setAutostart(next)
      .then(() => setAutostartSig(next))
      .catch(() => {});
  };

  const [cfg, setCfg] = createSignal<DesktopConfig>({
    download_path: null,
    language: null,
    max_quality: null,
    min_quality: null,
  });
  getConfig()
    .then(setCfg)
    .catch(() => {});
  const update = (patch: Partial<DesktopConfig>) => {
    const next = { ...cfg(), ...patch };
    setCfg(next);
    void setConfig(next).catch(() => {});
  };
  const empty = (v: string) => (v.trim() === "" ? null : v);

  let fileInput: HTMLInputElement | undefined;
  const onImport = async (e: Event) => {
    const f = (e.target as HTMLInputElement).files?.[0];
    if (f && (await importState(f))) location.reload();
  };

  const pickTheme = (t: Theme) => {
    applyTheme(t);
    setTheme(t);
  };
  const pickAccent = (a: Accent) => {
    applyAccent(a);
    setAccent(a);
  };
  const pickDensity = (d: Density) => {
    applyDensity(d);
    setDensity(d);
  };
  const pickScale = (s: Scale) => {
    applyScale(s);
    setScale(s);
  };

  return (
    <div class="wrap">
      <div class="row-h">
        <h2>Einstellungen</h2>
      </div>

      <div class="row-h" style={{ margin: "6px 2px 10px" }}>
        <h2 style={{ "font-size": "14px" }}>Erscheinungsbild</h2>
      </div>
      <div class="controls" style={{ "margin-bottom": "12px" }}>
        <For each={THEMES}>
          {(t) => (
            <button
              class="chip"
              classList={{ on: theme() === t.id }}
              onClick={() => pickTheme(t.id)}
            >
              {t.label}
            </button>
          )}
        </For>
      </div>
      <div class="controls" style={{ "margin-bottom": "12px" }}>
        <span class="sleep-label">Akzent</span>
        <For each={ACCENTS}>
          {(a) => (
            <button
              class="chip"
              classList={{ on: accent() === a.id }}
              onClick={() => pickAccent(a.id)}
            >
              {a.label}
            </button>
          )}
        </For>
      </div>
      <div class="controls" style={{ "margin-bottom": "12px" }}>
        <span class="sleep-label">Dichte</span>
        <For each={DENSITIES}>
          {(d) => (
            <button
              class="chip"
              classList={{ on: density() === d.id }}
              onClick={() => pickDensity(d.id)}
            >
              {d.label}
            </button>
          )}
        </For>
      </div>
      <div class="controls" style={{ "margin-bottom": "18px" }}>
        <span class="sleep-label">Größe</span>
        <For each={SCALES}>
          {(s) => (
            <button
              class="chip"
              classList={{ on: scale() === s.id }}
              onClick={() => pickScale(s.id)}
            >
              {s.label}
            </button>
          )}
        </For>
      </div>

      <div class="row-h" style={{ margin: "6px 2px 10px" }}>
        <h2 style={{ "font-size": "14px" }}>Daten</h2>
      </div>
      <div class="controls" style={{ "margin-bottom": "18px" }}>
        <button class="chip" onClick={exportState}>
          Sichern (Export)
        </button>
        <button class="chip" onClick={() => fileInput?.click()}>
          Wiederherstellen (Import)
        </button>
        <input
          ref={fileInput}
          type="file"
          accept="application/json"
          style={{ display: "none" }}
          onChange={(e) => void onImport(e)}
        />
      </div>

      <div class="row-h" style={{ margin: "6px 2px 10px" }}>
        <h2 style={{ "font-size": "14px" }}>System</h2>
      </div>
      <div class="controls" style={{ "margin-bottom": "18px" }}>
        <button class="chip" classList={{ on: autostart() }} onClick={toggleAutostart}>
          {autostart() ? "★ Autostart an" : "☆ Beim Login starten"}
        </button>
      </div>

      <div class="row-h" style={{ margin: "6px 2px 10px" }}>
        <h2 style={{ "font-size": "14px" }}>Downloads</h2>
      </div>
      <div class="panel" style={{ padding: "16px", "margin-bottom": "18px" }}>
        <div class="field">
          <label>Ordner</label>
          <input
            class="cfg-input"
            placeholder="Standard (~/Downloads)"
            value={cfg().download_path ?? ""}
            onChange={(e) => update({ download_path: empty(e.currentTarget.value) })}
          />
        </div>
        <div class="field">
          <label>Sprache</label>
          <select
            class="cfg-input"
            value={cfg().language ?? ""}
            onChange={(e) => update({ language: empty(e.currentTarget.value) })}
          >
            <option value="">Standard</option>
            <For each={LANGUAGES}>{(l) => <option value={l}>{l}</option>}</For>
          </select>
        </div>
        <div class="field">
          <label>Max. Qualität</label>
          <select
            class="cfg-input"
            value={cfg().max_quality ?? ""}
            onChange={(e) => update({ max_quality: empty(e.currentTarget.value) })}
          >
            <option value="">Standard</option>
            <For each={QUALITIES}>{(q) => <option value={q}>{q}</option>}</For>
          </select>
        </div>
        <div class="field">
          <label>Min. Qualität</label>
          <select
            class="cfg-input"
            value={cfg().min_quality ?? ""}
            onChange={(e) => update({ min_quality: empty(e.currentTarget.value) })}
          >
            <option value="">Standard</option>
            <For each={QUALITIES}>{(q) => <option value={q}>{q}</option>}</For>
          </select>
        </div>
      </div>

      <Show when={settings()} fallback={<div class="spinner" />}>
        {(s) => (
          <div class="panel">
            <dl class="kv">
              <dt>Version</dt>
              <dd>{info()?.version ?? "–"}</dd>
              <dt>Download-Ordner</dt>
              <dd>{s().download_path}</dd>
              <dt>Sprache</dt>
              <dd>{s().language}</dd>
              <dt>Qualität</dt>
              <dd>
                {s().min_quality} – {s().max_quality}
              </dd>
              <dt>Hoster-Reihenfolge</dt>
              <dd>{s().hosters.join(", ")}</dd>
              <dt>Nativer HLS-Download</dt>
              <dd>{s().native_hls ? "an" : "aus"}</dd>
              <dt>ffmpeg-frei (MKV-Remux)</dt>
              <dd>{s().no_ffmpeg ? "an" : "aus"}</dd>
            </dl>
          </div>
        )}
      </Show>

      <p class="note">
        Die Werte oben (Ordner, Sprache, Qualität) überschreiben die Defaults und werden
        gespeichert. Hoster-Reihenfolge und die weiteren Werte stammen aus den{" "}
        <code>VINDEX_*</code>-Variablen und <code>.env</code>.
      </p>

      <Show when={settings.error}>
        <div class="panel" style={{ padding: "14px 16px", color: "var(--err)" }}>
          Fehler: {String(settings.error)}
        </div>
      </Show>
    </div>
  );
}
