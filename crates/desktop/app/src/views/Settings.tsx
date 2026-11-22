import { createResource, createSignal, For, Show, type JSX } from "solid-js";
import { appSettings, appInfo } from "../lib/api";
import { applyTheme, getTheme, type Theme } from "../lib/theme";

const THEMES: { id: Theme; label: string }[] = [
  { id: "dark", label: "Dunkel" },
  { id: "light", label: "Hell" },
  { id: "amoled", label: "AMOLED" },
];

// Settings: zeigt die effektiven Laufzeit-Einstellungen (VINDEX_* / .env), live aus dem Kern.
export function Settings(): JSX.Element {
  const [settings] = createResource(appSettings);
  const [info] = createResource(appInfo);
  const [theme, setTheme] = createSignal<Theme>(getTheme());

  const pickTheme = (t: Theme) => {
    applyTheme(t);
    setTheme(t);
  };

  return (
    <div class="wrap">
      <div class="row-h">
        <h2>Einstellungen</h2>
      </div>

      <div class="row-h" style={{ margin: "6px 2px 10px" }}>
        <h2 style={{ "font-size": "14px" }}>Erscheinungsbild</h2>
      </div>
      <div class="controls" style={{ "margin-bottom": "18px" }}>
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
        Nur-Lese-Ansicht. Bearbeitbare Einstellungen, Themes, Netzwerk/Proxy und Shortcuts
        folgen. Werte stammen aus den <code>VINDEX_*</code>-Variablen und{" "}
        <code>.env</code>.
      </p>

      <Show when={settings.error}>
        <div class="panel" style={{ padding: "14px 16px", color: "var(--err)" }}>
          Fehler: {String(settings.error)}
        </div>
      </Show>
    </div>
  );
}
