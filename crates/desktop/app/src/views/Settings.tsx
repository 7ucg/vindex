import { createResource, createSignal, For, Show, type JSX } from "solid-js";
import {
  appSettings,
  appInfo,
  autostartEnabled,
  setAutostart,
  getConfig,
  setConfig,
  setBandwidthLimit,
  setDownloadDelay,
  pickDownloadFolder,
  recentLogs,
  networkStatus,
  setImpersonate,
  setObscura,
  setProxy,
  indexInfo,
  checkUpdate,
  installUpdate,
  type DesktopConfig,
  type NetworkStatus,
} from "../lib/api";

import { exportState, importState } from "../lib/backup";
import {
  ACTIONS,
  ACTION_LABELS,
  bindings,
  setBinding,
  resetBindings,
  type Action,
} from "../lib/keybindings";
import { incognito, setIncognito } from "../lib/private";

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
// Eine Einstellungs-Sektion als Karte mit Titel und kurzer Erklärung.
function Section(props: { title: string; desc?: string; children: JSX.Element }): JSX.Element {
  return (
    <section class="set-card">
      <div class="set-card-head">
        <h3>{props.title}</h3>
        <Show when={props.desc}>{(d) => <p class="set-card-desc">{d()}</p>}</Show>
      </div>
      <div class="set-card-body">{props.children}</div>
    </section>
  );
}

export function Settings(): JSX.Element {
  const [settings] = createResource(appSettings);
  const [info] = createResource(appInfo);
  const [idx] = createResource(indexInfo);
  const PORTAL_LABEL: Record<string, string> = {
    aniworld: "aniworld.to",
    serienstream: "serienstream.to",
    filmo: "filmo.to",
  };

  // Player-Tastenkürzel umbelegen (Overrides lokal).
  const [bindVer, setBindVer] = createSignal(0);
  const [capturing, setCapturing] = createSignal<Action | null>(null);
  // Reaktiv auf Umbelegungen reagieren (bindings() liest Modul-State).
  const binds = () => {
    bindVer();
    return bindings();
  };
  const keyLabel = (k: string) => (k === " " ? "Leertaste" : k === "" ? "–" : k.toUpperCase());
  const startCapture = (action: Action) => {
    setCapturing(action);
    const onKey = (e: KeyboardEvent) => {
      if (["Shift", "Control", "Alt", "Meta"].includes(e.key)) return;
      e.preventDefault();
      e.stopPropagation();
      window.removeEventListener("keydown", onKey, true);
      setCapturing(null);
      if (e.key === "Escape") return;
      if (/^Arrow/.test(e.key) || /^[0-9]$/.test(e.key)) return;
      setBinding(action, e.key);
      setBindVer((n) => n + 1);
    };
    window.addEventListener("keydown", onKey, true);
  };
  const resetKeys = () => {
    resetBindings();
    setBindVer((n) => n + 1);
  };

  // Manuelle Update-Prüfung (zusätzlich zum Start-Check).
  const [updateMsg, setUpdateMsg] = createSignal<string | null>(null);
  const [pendingVer, setPendingVer] = createSignal<string | null>(null);
  const checkForUpdate = async () => {
    setUpdateMsg("Suche …");
    setPendingVer(null);
    try {
      const u = await checkUpdate();
      if (!u) {
        setUpdateMsg("Du bist aktuell.");
        return;
      }
      setUpdateMsg(`Update ${u.version} verfügbar (aktuell ${u.current}).`);
      setPendingVer(u.version);
    } catch {
      setUpdateMsg("Keine Updates abrufbar (Proxy/obscura, Signatur oder Netz).");
    }
  };
  const doUpdate = () => {
    const v = pendingVer();
    if (!v) return;
    setUpdateMsg("Installiere & starte neu …");
    void installUpdate(v).catch((e) => setUpdateMsg(`Fehler: ${String(e)}`));
  };
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
    bandwidth_kib: null,
    download_delay_ms: null,
    native_hls: null,
    no_ffmpeg: null,
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
  const browseFolder = async () => {
    const p = await pickDownloadFolder().catch(() => null);
    if (p) update({ download_path: p });
  };
  // Tempolimit getrennt: eigener Command, der den Laufzeit-Limiter setzt und persistiert.
  const updateBandwidth = (v: string) => {
    const kib = Math.max(0, Math.floor(Number(v) || 0));
    setCfg({ ...cfg(), bandwidth_kib: kib === 0 ? null : kib });
    void setBandwidthLimit(kib).catch(() => {});
  };
  const updateDelay = (v: string) => {
    const ms = Math.max(0, Math.floor(Number(v) || 0));
    setCfg({ ...cfg(), download_delay_ms: ms === 0 ? null : ms });
    void setDownloadDelay(ms).catch(() => {});
  };

  // Netzwerk: DNS fest 1.1.1.1; Impersonate (Stufe 2) + obscura (Stufe 3) live, Proxy nach Neustart.
  const [net, setNet] = createSignal<NetworkStatus | null>(null);
  const [obscuraInput, setObscuraInput] = createSignal("");
  const [proxyInput, setProxyInput] = createSignal("");
  const reloadNet = () => {
    networkStatus()
      .then((n) => {
        setNet(n);
        setObscuraInput(n.obscura_path ?? "");
        setProxyInput(n.proxy ?? "");
      })
      .catch(() => {});
  };
  reloadNet();
  const toggleImpersonate = () => {
    const n = net();
    if (!n) return;
    void setImpersonate(!n.impersonate)
      .then(reloadNet)
      .catch(() => {});
  };
  const saveObscura = () => {
    void setObscura(obscuraInput().trim() || null)
      .then(reloadNet)
      .catch(() => {});
  };
  const saveProxy = () => {
    void setProxy(proxyInput().trim() || null)
      .then(reloadNet)
      .catch(() => {});
  };

  let fileInput: HTMLInputElement | undefined;
  const onImport = async (e: Event) => {
    const f = (e.target as HTMLInputElement).files?.[0];
    if (f && (await importState(f))) location.reload();
  };

  // Diagnose: die letzten Log-Zeilen (redigiert) anzeigen, kopieren, exportieren.
  const [logs, setLogs] = createSignal<string[]>([]);
  const [logsShown, setLogsShown] = createSignal(false);
  const loadLogs = async () => {
    try {
      setLogs(await recentLogs());
      setLogsShown(true);
    } catch {
      /* egal */
    }
  };
  const copyLogs = () => {
    void navigator.clipboard?.writeText(logs().join("\n")).catch(() => {});
  };
  const exportLogs = () => {
    const blob = new Blob([logs().join("\n")], { type: "text/plain" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `vindex-log-${new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-")}.txt`;
    a.click();
    URL.revokeObjectURL(a.href);
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
    <div class="wrap settings">
      <div class="row-h">
        <h2>Einstellungen</h2>
      </div>

      <Section
        title="Erscheinungsbild"
        desc="Thema, Akzentfarbe, Dichte und Darstellungsgröße der App."
      >
        <div class="controls">
          <span class="sleep-label">Thema</span>
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

      </Section>

      <Section title="Daten" desc="Einstellungen und Merklisten sichern oder wiederherstellen.">
        <div class="controls">
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

      </Section>

      <Section
        title="Index"
        desc="Lokale Katalog-DBs (offline). Werden beim Start entpackt und von hier gelesen."
      >
        <Show when={idx()} fallback={<div class="spinner" />}>
          {(i) => (
            <>
              <div class="field">
                <label>Ordner</label>
                <input class="cfg-input" value={i().dir} readonly />
              </div>
              <dl class="kv">
                <For each={i().portals}>
                  {(p) => (
                    <>
                      <dt>{PORTAL_LABEL[p.portal] ?? p.portal}</dt>
                      <dd>{p.titles === null ? "nicht vorhanden" : `${p.titles} Titel`}</dd>
                    </>
                  )}
                </For>
              </dl>
            </>
          )}
        </Show>
      </Section>

      <Section title="Privatsphäre" desc="Inkognito schreibt in dieser Sitzung nichts mit.">
        <div class="controls" style={{ "margin-bottom": "6px" }}>
        <button
          class="chip"
          classList={{ on: incognito() }}
          onClick={() => setIncognito(!incognito())}
        >
          {incognito() ? "● Inkognito an" : "○ Inkognito"}
        </button>
      </div>
      <p class="note" style={{ "margin-bottom": "18px" }}>
        Inkognito schreibt für diese Sitzung nichts mit (kein Fortschritt/„Weiterschauen", kein
        Download-Verlauf). Gilt nur bis zum nächsten Start; Merken bleibt möglich.
      </p>

      </Section>

      <Section title="System" desc="Autostart beim Login und Updates aus den GitHub-Releases.">
        <div class="controls">
          <button class="chip" classList={{ on: autostart() }} onClick={toggleAutostart}>
            {autostart() ? "★ Autostart an" : "☆ Beim Login starten"}
          </button>
          <button class="chip" onClick={() => void checkForUpdate()}>
            Nach Updates suchen
          </button>
          <Show when={pendingVer()}>
            <button class="chip" onClick={doUpdate}>
              Installieren & neu starten
            </button>
          </Show>
        </div>
        <Show when={updateMsg()}>
          <p class="note" style={{ margin: "8px 0 0" }}>
            {updateMsg()}
          </p>
        </Show>
      </Section>

      <Section
        title="Tastenkürzel"
        desc="Player-Tasten umbelegen. Pfeile (±5/±30 s), Ziffern (springen) und Esc bleiben fest."
      >
        <div class="controls" style={{ "margin-bottom": "10px" }}>
          <button class="chip" onClick={resetKeys}>
            Auf Standard zurücksetzen
          </button>
        </div>
        <div class="set-keys">
          <For each={ACTIONS}>
            {(a) => (
              <div class="set-key-row">
                <span class="set-key-label">{ACTION_LABELS[a]}</span>
                <button
                  class="chip"
                  classList={{ on: capturing() === a }}
                  onClick={() => startCapture(a)}
                >
                  {capturing() === a ? "Taste drücken …" : keyLabel(binds()[a])}
                </button>
              </div>
            )}
          </For>
        </div>
      </Section>

      <Section
        title="Downloads"
        desc="Zielordner, Sprache, Qualität, Tempolimit und Lade-Verfahren."
      >
        <div class="set-fields">
        <div class="field">
          <label>Ordner</label>
          <div style={{ display: "flex", gap: "8px" }}>
            <input
              class="cfg-input"
              placeholder="Standard (~/Downloads)"
              value={cfg().download_path ?? ""}
              onChange={(e) => update({ download_path: empty(e.currentTarget.value) })}
            />
            <button class="chip" onClick={() => void browseFolder()}>
              Durchsuchen …
            </button>
          </div>
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
        <div class="field">
          <label>Tempolimit (KiB/s, 0 = unbegrenzt)</label>
          <input
            class="cfg-input"
            type="number"
            min="0"
            step="256"
            placeholder="0"
            value={cfg().bandwidth_kib ?? ""}
            onChange={(e) => updateBandwidth(e.currentTarget.value)}
          />
        </div>
        <div class="field">
          <label>Abstand zwischen Download-Starts (ms, 0 = aus)</label>
          <input
            class="cfg-input"
            type="number"
            min="0"
            step="250"
            placeholder="0"
            value={cfg().download_delay_ms ?? ""}
            onChange={(e) => updateDelay(e.currentTarget.value)}
          />
        </div>
        <div class="field">
          <label>Lade-Verfahren</label>
          <div class="controls">
            <button
              class="chip"
              classList={{ on: cfg().native_hls ?? true }}
              onClick={() => update({ native_hls: !(cfg().native_hls ?? true) })}
            >
              {(cfg().native_hls ?? true) ? "● Nativer HLS-Download" : "○ Nativer HLS-Download"}
            </button>
            <button
              class="chip"
              classList={{ on: cfg().no_ffmpeg ?? true }}
              onClick={() => update({ no_ffmpeg: !(cfg().no_ffmpeg ?? true) })}
            >
              {(cfg().no_ffmpeg ?? true) ? "● Ohne ffmpeg (MKV-Remux)" : "○ ffmpeg zum Muxen"}
            </button>
          </div>
          <p class="note" style={{ margin: "6px 0 0" }}>
            Der ffmpeg-Pfad lädt direkt über das System-Netz (kein 1.1.1.1, kein Proxy, keine
            Drosselung). Bei gesetztem Proxy/obscura bleibt der native Pfad erzwungen.
          </p>
        </div>
      </div>

      </Section>

      <Section title="Netzwerk" desc="DNS fest auf 1.1.1.1; Anti-Bot-Stufen und Egress-Proxy.">
        <div class="set-fields">
        <div class="field">
          <label>DNS</label>
          <input class="cfg-input" value="1.1.1.1 (fest)" disabled />
        </div>
        <div class="controls" style={{ margin: "4px 0 12px" }}>
          <span class="sleep-label">Anti-Bot</span>
          <button class="chip" classList={{ on: net()?.impersonate }} onClick={toggleImpersonate}>
            {net()?.impersonate ? "● Stufe 2 (Impersonate) an" : "○ Stufe 2 (Impersonate)"}
          </button>
          <span class="sleep-label">
            {net()?.obscura ? "● Stufe 3 (obscura) aktiv" : "○ Stufe 3 aus"}
          </span>
        </div>
        <div class="field">
          <label>obscura-Pfad (Stufe 3, leer = aus)</label>
          <div style={{ display: "flex", gap: "8px" }}>
            <input
              class="cfg-input"
              placeholder="/pfad/zu/obscura"
              value={obscuraInput()}
              onInput={(e) => setObscuraInput(e.currentTarget.value)}
            />
            <button class="chip" onClick={saveObscura}>
              Speichern
            </button>
          </div>
        </div>
        <div class="field">
          <label>Egress-Proxy (wirkt nach Neustart, leer = keiner)</label>
          <div style={{ display: "flex", gap: "8px" }}>
            <input
              class="cfg-input"
              placeholder="http://host:port"
              value={proxyInput()}
              onInput={(e) => setProxyInput(e.currentTarget.value)}
            />
            <button class="chip" onClick={saveProxy}>
              Speichern
            </button>
          </div>
        </div>
      </div>
      <p class="note" style={{ "margin-bottom": "18px" }}>
        DNS ist fest auf 1.1.1.1 (nicht abschaltbar). Impersonate und obscura wirken sofort; ein
        Egress-Proxy wird beim nächsten Start übernommen. Mit gesetztem Proxy ist Stufe 3 (obscura)
        aus Sicherheitsgründen deaktiviert.
      </p>

      </Section>

      <Section title="Diagnose" desc="App-Logs ansehen (URLs sind redigiert).">
        <div class="controls" style={{ "margin-bottom": logsShown() ? "10px" : "18px" }}>
        <button class="chip" onClick={() => void loadLogs()}>
          {logsShown() ? "Aktualisieren" : "Logs anzeigen"}
        </button>
        <Show when={logsShown()}>
          <button class="chip" onClick={copyLogs}>
            Kopieren
          </button>
          <button class="chip" onClick={exportLogs}>
            Exportieren
          </button>
          <button class="chip" onClick={() => setLogsShown(false)}>
            Ausblenden
          </button>
        </Show>
      </div>
      <Show when={logsShown()}>
        <pre class="logbox">{logs().length ? logs().join("\n") : "Keine Log-Zeilen."}</pre>
      </Show>

      </Section>

      <Section title="Übersicht" desc="Aktuell wirksame Download-Einstellungen.">
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

      </Section>

      <Show when={settings.error}>
        <div class="panel" style={{ padding: "14px 16px", color: "var(--err)" }}>
          Fehler: {String(settings.error)}
        </div>
      </Show>
    </div>
  );
}
