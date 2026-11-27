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
  listSubscriptions,
  unsubscribe,
  checkSubscriptionsNow,
  downloadGateStatus,
  type DesktopConfig,
  type NetworkStatus,
  type Subscription,
  type GateStatus,
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
import { t, lang, setLang } from "../lib/i18n";

const LANGUAGES = ["German Dub", "German Sub", "English Sub", "English Dub"];
const QUALITIES = ["2160p", "1080p", "720p", "480p", "360p"];
// Anime4K-Modi (offizielle Shader-Ketten), wie im Kern geparst.
const ANIME4K_MODES = ["A", "B", "C", "A+A", "B+B", "C+A"];
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

// label ist ein i18n-Schlüssel, der zur Render-Zeit über t() aufgelöst wird (reaktiv auf Sprachwechsel).
const THEMES: { id: Theme; label: string }[] = [
  { id: "dark", label: "settings.theme.dark" },
  { id: "light", label: "settings.theme.light" },
  { id: "amoled", label: "settings.theme.amoled" },
];
const ACCENTS: { id: Accent; label: string }[] = [
  { id: "violett", label: "settings.accent.violett" },
  { id: "cyan", label: "settings.accent.cyan" },
  { id: "gruen", label: "settings.accent.gruen" },
  { id: "pink", label: "settings.accent.pink" },
  { id: "bernstein", label: "settings.accent.bernstein" },
];
const DENSITIES: { id: Density; label: string }[] = [
  { id: "normal", label: "settings.density.normal" },
  { id: "kompakt", label: "settings.density.kompakt" },
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
  const keyLabel = (k: string) =>
    k === " " ? t("settings.keys.space") : k === "" ? "–" : k.toUpperCase();
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
    setUpdateMsg(t("settings.system.searching"));
    setPendingVer(null);
    try {
      const u = await checkUpdate();
      if (!u) {
        setUpdateMsg(t("settings.system.upToDate"));
        return;
      }
      setUpdateMsg(t("settings.system.updateAvail", { version: u.version, current: u.current }));
      setPendingVer(u.version);
    } catch {
      setUpdateMsg(t("settings.system.updateFail"));
    }
  };
  const doUpdate = () => {
    const v = pendingVer();
    if (!v) return;
    setUpdateMsg(t("settings.system.installing"));
    void installUpdate(v).catch((e) => setUpdateMsg(t("settings.system.error", { e: String(e) })));
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
    external_player: null,
    anime4k: null,
    schedule_enabled: null,
    schedule_from: null,
    schedule_to: null,
    pause_on_battery: null,
    pause_on_metered: null,
    naming_template: null,
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

  // Zeitplan & Abos: Abo-Liste, manueller Durchlauf, Gate-Status (warum es wartet).
  const [subs, setSubs] = createSignal<Subscription[]>([]);
  const [gate, setGate] = createSignal<GateStatus | null>(null);
  const [checkMsg, setCheckMsg] = createSignal<string | null>(null);
  const reloadSubs = () => {
    listSubscriptions()
      .then(setSubs)
      .catch(() => {});
  };
  const reloadGate = () => {
    downloadGateStatus()
      .then(setGate)
      .catch(() => {});
  };
  reloadSubs();
  reloadGate();
  const removeSub = (url: string) => {
    void unsubscribe(url)
      .then(reloadSubs)
      .catch(() => {});
  };
  const checkNow = async () => {
    setCheckMsg(t("settings.schedule.searching"));
    try {
      const n = await checkSubscriptionsNow();
      setCheckMsg(
        n === 0
          ? t("settings.schedule.noneFound")
          : t("settings.schedule.queued", { n }),
      );
      reloadGate();
    } catch (e) {
      setCheckMsg(t("settings.schedule.error", { e: String(e) }));
    }
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
        <h2>{t("settings.title")}</h2>
      </div>

      <Section title={t("settings.appearance.title")} desc={t("settings.appearance.desc")}>
        <div class="controls">
          <span class="sleep-label">{t("settings.appearance.theme")}</span>
          <For each={THEMES}>
          {(th) => (
            <button
              class="chip"
              classList={{ on: theme() === th.id }}
              onClick={() => pickTheme(th.id)}
            >
              {t(th.label)}
            </button>
          )}
        </For>
      </div>
      <div class="controls" style={{ "margin-bottom": "12px" }}>
        <span class="sleep-label">{t("settings.appearance.accent")}</span>
        <For each={ACCENTS}>
          {(a) => (
            <button
              class="chip"
              classList={{ on: accent() === a.id }}
              onClick={() => pickAccent(a.id)}
            >
              {t(a.label)}
            </button>
          )}
        </For>
      </div>
      <div class="controls" style={{ "margin-bottom": "12px" }}>
        <span class="sleep-label">{t("settings.appearance.density")}</span>
        <For each={DENSITIES}>
          {(d) => (
            <button
              class="chip"
              classList={{ on: density() === d.id }}
              onClick={() => pickDensity(d.id)}
            >
              {t(d.label)}
            </button>
          )}
        </For>
      </div>
      <div class="controls" style={{ "margin-bottom": "12px" }}>
        <span class="sleep-label">{t("settings.appearance.scale")}</span>
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
      <div class="controls" style={{ "margin-bottom": "18px" }}>
        <span class="sleep-label">{t("settings.appearance.language")}</span>
        <button class="chip" classList={{ on: lang() === "de" }} onClick={() => setLang("de")}>
          Deutsch
        </button>
        <button class="chip" classList={{ on: lang() === "en" }} onClick={() => setLang("en")}>
          English
        </button>
      </div>

      </Section>

      <Section title={t("settings.data.title")} desc={t("settings.data.desc")}>
        <div class="controls">
        <button class="chip" onClick={exportState}>
          {t("settings.data.backup")}
        </button>
        <button class="chip" onClick={() => fileInput?.click()}>
          {t("settings.data.restore")}
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

      <Section title={t("settings.index.title")} desc={t("settings.index.desc")}>
        <Show when={idx()} fallback={<div class="spinner" />}>
          {(i) => (
            <>
              <div class="field">
                <label>{t("settings.index.folder")}</label>
                <input class="cfg-input" value={i().dir} readonly />
              </div>
              <dl class="kv">
                <For each={i().portals}>
                  {(p) => (
                    <>
                      <dt>{PORTAL_LABEL[p.portal] ?? p.portal}</dt>
                      <dd>
                        {p.titles === null
                          ? t("settings.index.missing")
                          : t("settings.index.titles", { n: p.titles })}
                      </dd>
                    </>
                  )}
                </For>
              </dl>
            </>
          )}
        </Show>
      </Section>

      <Section title={t("settings.privacy.title")} desc={t("settings.privacy.desc")}>
        <div class="controls" style={{ "margin-bottom": "6px" }}>
        <button
          class="chip"
          classList={{ on: incognito() }}
          onClick={() => setIncognito(!incognito())}
        >
          {incognito() ? t("settings.privacy.on") : t("settings.privacy.off")}
        </button>
      </div>
      <p class="note" style={{ "margin-bottom": "18px" }}>
        {t("settings.privacy.note")}
      </p>

      </Section>

      <Section title={t("settings.system.title")} desc={t("settings.system.desc")}>
        <div class="controls">
          <button class="chip" classList={{ on: autostart() }} onClick={toggleAutostart}>
            {autostart() ? t("settings.system.autostartOn") : t("settings.system.autostartOff")}
          </button>
          <button class="chip" onClick={() => void checkForUpdate()}>
            {t("settings.system.checkUpdate")}
          </button>
          <Show when={pendingVer()}>
            <button class="chip" onClick={doUpdate}>
              {t("settings.system.installRestart")}
            </button>
          </Show>
        </div>
        <Show when={updateMsg()}>
          <p class="note" style={{ margin: "8px 0 0" }}>
            {updateMsg()}
          </p>
        </Show>
      </Section>

      <Section title={t("settings.keys.title")} desc={t("settings.keys.desc")}>
        <div class="controls" style={{ "margin-bottom": "10px" }}>
          <button class="chip" onClick={resetKeys}>
            {t("settings.keys.reset")}
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
                  {capturing() === a ? t("settings.keys.press") : keyLabel(binds()[a])}
                </button>
              </div>
            )}
          </For>
        </div>
      </Section>

      <Section title={t("settings.downloads.title")} desc={t("settings.downloads.desc")}>
        <div class="set-fields">
        <div class="field">
          <label>{t("settings.downloads.folder")}</label>
          <div style={{ display: "flex", gap: "8px" }}>
            <input
              class="cfg-input"
              placeholder={t("settings.downloads.folderPlaceholder")}
              value={cfg().download_path ?? ""}
              onChange={(e) => update({ download_path: empty(e.currentTarget.value) })}
            />
            <button class="chip" onClick={() => void browseFolder()}>
              {t("settings.downloads.browse")}
            </button>
          </div>
        </div>
        <div class="field">
          <label>{t("settings.downloads.language")}</label>
          <select
            class="cfg-input"
            value={cfg().language ?? ""}
            onChange={(e) => update({ language: empty(e.currentTarget.value) })}
          >
            <option value="">{t("settings.downloads.default")}</option>
            <For each={LANGUAGES}>{(l) => <option value={l}>{l}</option>}</For>
          </select>
        </div>
        <div class="field">
          <label>{t("settings.downloads.maxQuality")}</label>
          <select
            class="cfg-input"
            value={cfg().max_quality ?? ""}
            onChange={(e) => update({ max_quality: empty(e.currentTarget.value) })}
          >
            <option value="">{t("settings.downloads.default")}</option>
            <For each={QUALITIES}>{(q) => <option value={q}>{q}</option>}</For>
          </select>
        </div>
        <div class="field">
          <label>{t("settings.downloads.minQuality")}</label>
          <select
            class="cfg-input"
            value={cfg().min_quality ?? ""}
            onChange={(e) => update({ min_quality: empty(e.currentTarget.value) })}
          >
            <option value="">{t("settings.downloads.default")}</option>
            <For each={QUALITIES}>{(q) => <option value={q}>{q}</option>}</For>
          </select>
        </div>
        <div class="field">
          <label>{t("settings.downloads.bandwidth")}</label>
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
          <label>{t("settings.downloads.delay")}</label>
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
          <label>Dateinamen-Muster</label>
          <input
            class="cfg-input"
            placeholder="{title} S{s2}E{e2}"
            value={cfg().naming_template ?? ""}
            onChange={(e) => update({ naming_template: empty(e.currentTarget.value) })}
          />
          <p class="note" style={{ margin: "6px 0 0" }}>
            Leer = Standard. Platzhalter: {"{title}"} {"{season}"} {"{s2}"} {"{episode}"} {"{e2}"}
            {" "}(s2/e2 = 2-stellig). Pfadtrenner werden entfernt.
          </p>
        </div>
        <div class="field">
          <label>{t("settings.downloads.method")}</label>
          <div class="controls">
            <button
              class="chip"
              classList={{ on: cfg().native_hls ?? true }}
              onClick={() => update({ native_hls: !(cfg().native_hls ?? true) })}
            >
              {((cfg().native_hls ?? true) ? "● " : "○ ") + t("settings.downloads.nativeHls")}
            </button>
            <button
              class="chip"
              classList={{ on: cfg().no_ffmpeg ?? true }}
              onClick={() => update({ no_ffmpeg: !(cfg().no_ffmpeg ?? true) })}
            >
              {(cfg().no_ffmpeg ?? true)
                ? "● " + t("settings.downloads.noFfmpegOn")
                : "○ " + t("settings.downloads.noFfmpegOff")}
            </button>
          </div>
          <p class="note" style={{ margin: "6px 0 0" }}>
            {t("settings.downloads.note")}
          </p>
        </div>
      </div>

      </Section>

      <Section title={t("settings.schedule.title")} desc={t("settings.schedule.desc")}>
        <div class="controls" style={{ "margin-bottom": "12px" }}>
          <button
            class="chip"
            classList={{ on: cfg().schedule_enabled ?? false }}
            onClick={() => update({ schedule_enabled: !(cfg().schedule_enabled ?? false) })}
          >
            {(cfg().schedule_enabled ?? false)
              ? t("settings.schedule.on")
              : t("settings.schedule.off")}
          </button>
        </div>
        <div class="set-fields">
          <div class="field">
            <label>{t("settings.schedule.from")}</label>
            <input
              class="cfg-input"
              type="time"
              style={{ "max-width": "140px" }}
              value={cfg().schedule_from ?? ""}
              onChange={(e) => update({ schedule_from: empty(e.currentTarget.value) })}
            />
          </div>
          <div class="field">
            <label>{t("settings.schedule.to")}</label>
            <input
              class="cfg-input"
              type="time"
              style={{ "max-width": "140px" }}
              value={cfg().schedule_to ?? ""}
              onChange={(e) => update({ schedule_to: empty(e.currentTarget.value) })}
            />
          </div>
          <div class="field">
            <label>{t("settings.schedule.pause")}</label>
            <div class="controls">
              <button
                class="chip"
                classList={{ on: cfg().pause_on_battery ?? false }}
                onClick={() => update({ pause_on_battery: !(cfg().pause_on_battery ?? false) })}
              >
                {((cfg().pause_on_battery ?? false) ? "● " : "○ ") + t("settings.schedule.onBattery")}
              </button>
              <button
                class="chip"
                classList={{ on: cfg().pause_on_metered ?? false }}
                onClick={() => update({ pause_on_metered: !(cfg().pause_on_metered ?? false) })}
              >
                {((cfg().pause_on_metered ?? false) ? "● " : "○ ") + t("settings.schedule.onMetered")}
              </button>
            </div>
          </div>
        </div>

        <div class="controls" style={{ margin: "12px 0" }}>
          <button class="chip" onClick={() => void checkNow()}>
            {t("settings.schedule.checkNow")}
          </button>
          <Show when={checkMsg()}>
            <span class="sleep-label">{checkMsg()}</span>
          </Show>
        </div>

        <Show when={gate()?.held_reason}>
          {(reason) => (
            <p class="note" style={{ margin: "0 0 12px" }}>
              {t("settings.schedule.waiting", { reason: reason() })}
            </p>
          )}
        </Show>

        <Show
          when={subs().length > 0}
          fallback={
            <p class="note" style={{ margin: 0 }}>
              {t("settings.schedule.noSubs")}
            </p>
          }
        >
          <div class="panel">
            <For each={subs()}>
              {(s) => (
                <div class="dl" title={s.url}>
                  <div class="info">
                    <div class="n">{s.title}</div>
                    <div class="sub">
                      {[s.lang, s.hoster, s.maxQuality].filter(Boolean).join(" · ") ||
                        t("settings.schedule.defaultSelection")}
                    </div>
                  </div>
                  <div class="act">
                    <button class="chip" onClick={() => removeSub(s.url)}>
                      {t("common.remove")}
                    </button>
                  </div>
                </div>
              )}
            </For>
          </div>
        </Show>
      </Section>

      <Section title={t("settings.playback.title")} desc={t("settings.playback.desc")}>
        <div class="controls" style={{ "margin-bottom": "12px" }}>
          <span class="sleep-label">{t("settings.playback.player")}</span>
          <button
            class="chip"
            classList={{ on: (cfg().external_player ?? "") === "mpv" }}
            onClick={() =>
              update({ external_player: (cfg().external_player ?? "") === "mpv" ? null : "mpv" })
            }
          >
            {((cfg().external_player ?? "") === "mpv" ? "● " : "○ ") + t("settings.playback.mpv")}
          </button>
        </div>
        <div class="field">
          <label>{t("settings.playback.anime4k")}</label>
          <select
            class="cfg-input"
            style={{ "max-width": "220px" }}
            value={cfg().anime4k ?? ""}
            onChange={(e) => update({ anime4k: empty(e.currentTarget.value) })}
          >
            <option value="">{t("settings.playback.off")}</option>
            <For each={ANIME4K_MODES}>{(m) => <option value={m}>{m}</option>}</For>
          </select>
        </div>
        <p class="note" style={{ margin: "6px 0 0" }}>
          {t("settings.playback.note1")} <code>VINDEX_ANIME4K_DIR</code>
          {t("settings.playback.note2")}
        </p>
      </Section>

      <Section title={t("settings.network.title")} desc={t("settings.network.desc")}>
        <div class="set-fields">
        <div class="field">
          <label>{t("settings.network.dns")}</label>
          <input class="cfg-input" value={t("settings.network.dnsValue")} disabled />
        </div>
        <div class="controls" style={{ margin: "4px 0 12px" }}>
          <span class="sleep-label">{t("settings.network.antibot")}</span>
          <button class="chip" classList={{ on: net()?.impersonate }} onClick={toggleImpersonate}>
            {net()?.impersonate
              ? t("settings.network.impersonateOn")
              : t("settings.network.impersonateOff")}
          </button>
          <span class="sleep-label">
            {net()?.obscura
              ? t("settings.network.obscuraOn")
              : t("settings.network.obscuraOff")}
          </span>
        </div>
        <div class="field">
          <label>{t("settings.network.obscuraPath")}</label>
          <div style={{ display: "flex", gap: "8px" }}>
            <input
              class="cfg-input"
              placeholder={t("settings.network.obscuraPlaceholder")}
              value={obscuraInput()}
              onInput={(e) => setObscuraInput(e.currentTarget.value)}
            />
            <button class="chip" onClick={saveObscura}>
              {t("common.save")}
            </button>
          </div>
        </div>
        <div class="field">
          <label>{t("settings.network.proxy")}</label>
          <div style={{ display: "flex", gap: "8px" }}>
            <input
              class="cfg-input"
              placeholder="http://host:port"
              value={proxyInput()}
              onInput={(e) => setProxyInput(e.currentTarget.value)}
            />
            <button class="chip" onClick={saveProxy}>
              {t("common.save")}
            </button>
          </div>
        </div>
      </div>
      <p class="note" style={{ "margin-bottom": "18px" }}>
        {t("settings.network.note")}
      </p>

      </Section>

      <Section title={t("settings.diag.title")} desc={t("settings.diag.desc")}>
        <div class="controls" style={{ "margin-bottom": logsShown() ? "10px" : "18px" }}>
        <button class="chip" onClick={() => void loadLogs()}>
          {logsShown() ? t("settings.diag.refresh") : t("settings.diag.showLogs")}
        </button>
        <Show when={logsShown()}>
          <button class="chip" onClick={copyLogs}>
            {t("common.copy")}
          </button>
          <button class="chip" onClick={exportLogs}>
            {t("common.export")}
          </button>
          <button class="chip" onClick={() => setLogsShown(false)}>
            {t("common.hide")}
          </button>
        </Show>
      </div>
      <Show when={logsShown()}>
        <pre class="logbox">{logs().length ? logs().join("\n") : t("settings.diag.noLogs")}</pre>
      </Show>

      </Section>

      <Section title={t("settings.overview.title")} desc={t("settings.overview.desc")}>
      <Show when={settings()} fallback={<div class="spinner" />}>
        {(s) => (
          <div class="panel">
            <dl class="kv">
              <dt>{t("settings.overview.version")}</dt>
              <dd>{info()?.version ?? "–"}</dd>
              <dt>{t("settings.overview.downloadFolder")}</dt>
              <dd>{s().download_path}</dd>
              <dt>{t("settings.overview.language")}</dt>
              <dd>{s().language}</dd>
              <dt>{t("settings.overview.quality")}</dt>
              <dd>
                {s().min_quality} – {s().max_quality}
              </dd>
              <dt>{t("settings.overview.hosterOrder")}</dt>
              <dd>{s().hosters.join(", ")}</dd>
              <dt>{t("settings.overview.nativeHls")}</dt>
              <dd>{s().native_hls ? t("settings.overview.on") : t("settings.overview.off")}</dd>
              <dt>{t("settings.overview.noFfmpeg")}</dt>
              <dd>{s().no_ffmpeg ? t("settings.overview.on") : t("settings.overview.off")}</dd>
            </dl>
          </div>
        )}
      </Show>

      <p class="note">
        {t("settings.overview.note1")} <code>VINDEX_*</code>
        {t("settings.overview.note2")} <code>.env</code>
        {t("settings.overview.note3")}
      </p>

      </Section>

      <Show when={settings.error}>
        <div class="panel" style={{ padding: "14px 16px", color: "var(--err)" }}>
          {t("settings.loadError", { e: String(settings.error) })}
        </div>
      </Show>
    </div>
  );
}
