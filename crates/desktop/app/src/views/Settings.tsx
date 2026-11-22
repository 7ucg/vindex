import { createResource, Show, type JSX } from "solid-js";
import { appSettings, appInfo } from "../lib/api";

// Settings: zeigt die effektiven Laufzeit-Einstellungen (VINDEX_* / .env), live aus dem Kern.
export function Settings(): JSX.Element {
  const [settings] = createResource(appSettings);
  const [info] = createResource(appInfo);

  return (
    <div class="wrap">
      <div class="row-h">
        <h2>Einstellungen</h2>
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
