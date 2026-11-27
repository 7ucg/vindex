// Typisierte Brücke zu den Tauri-Commands (crates/desktop/app/src-tauri/src/lib.rs).
// Jede Funktion mappt 1:1 auf ein #[tauri::command]; das Frontend ruft nie direkt
// ins Netz, sondern immer über diese Schicht in den Rust-Kern.
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface AppInfo {
  name: string;
  version: string;
}

export interface SearchHit {
  title: string;
  url: string;
  source?: "db" | "live";
}

/** Portal-URL eines Titels per Name (für Bibliotheks-Cover). */
export function titleUrlByName(name: string): Promise<string | null> {
  return invoke<string | null>("title_url_by_name", { name });
}

/** Herkunft eines Titels: "db" (lokaler Index) oder "live". */
export function dataSource(url: string): Promise<"db" | "live"> {
  return invoke<"db" | "live">("data_source", { url });
}

export interface IndexPortal {
  portal: string;
  titles: number | null;
}
export interface IndexInfo {
  dir: string;
  portals: IndexPortal[];
}

/** Wo die Index-DBs liegen und wie viele Titel je Portal. */
export function indexInfo(): Promise<IndexInfo> {
  return invoke<IndexInfo>("index_info");
}

export interface AppSettings {
  download_path: string;
  language: string;
  max_quality: string;
  min_quality: string;
  hosters: string[];
  native_hls: boolean;
  no_ffmpeg: boolean;
}

export function appInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

/** Fenster in die untere rechte Ecke schnappen (Mini-Player). */
export function snapBottomRight(): Promise<void> {
  return invoke("snap_bottom_right");
}

/** System-Locale (z. B. "de-DE") für i18n; null, wenn nicht ermittelbar. */
export function osLocale(): Promise<string | null> {
  return invoke<string | null>("os_locale");
}

export interface UpdateInfo {
  version: string;
  current: string;
  notes: string | null;
}

/** Prüft die GitHub-Releases auf ein neueres Build (null = aktuell). */
export function checkUpdate(): Promise<UpdateInfo | null> {
  return invoke<UpdateInfo | null>("check_update");
}

/** Lädt + installiert die bestätigte Update-Version und startet die App neu. */
export function installUpdate(version: string): Promise<void> {
  return invoke("install_update", { version });
}

export function search(keyword: string): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("search", { keyword });
}

export function catalog(portal: string): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("catalog", { portal });
}

/** Nach Genre oder Jahr gefiltert; ohne Filter = voller Katalog. */
export function discover(
  portal: string,
  genre?: string,
  year?: number,
  language?: string,
): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("discover", {
    portal,
    genre: genre || null,
    year: year ?? null,
    language: language || null,
  });
}

export interface Episode {
  url: string;
  title: string;
  season: number;
  index_in_season: number;
}

export interface PlaybackSource {
  url: string;
  is_hls: boolean;
}

export function resolvePlayback(url: string, lang?: string): Promise<PlaybackSource> {
  return invoke<PlaybackSource>("resolve_playback", { url, lang: lang ?? null });
}

export interface LangOption {
  value: string;
  label: string;
  sub_only: boolean;
}

/** Tatsächlich für diesen Titel verfügbare Sprachen (inkl. „nur Untertitel"). */
export function availableLanguages(url: string): Promise<LangOption[]> {
  return invoke<LangOption[]>("available_languages", { url });
}

export interface SkipInterval {
  start: number;
  end: number;
  kind: string; // "op" (Intro) | "ed" (Outro)
}

export function aniskip(url: string, season: number, episode: number): Promise<SkipInterval[]> {
  return invoke<SkipInterval[]>("aniskip", { url, season, episode });
}

export interface TitleMeta {
  poster: string | null;
  description: string | null;
  year: string | null;
  fsk: number | null;
  country: string | null;
  imdb: string | null;
  genres: string[];
  directors: string[];
  actors: string[];
  producers: string[];
}

export function titleMeta(url: string): Promise<TitleMeta> {
  return invoke<TitleMeta>("title_meta", { url });
}

/** Öffnet einen externen Link (nur https, geprüfte Hosts) im System-Browser. */
export function openExternal(url: string): Promise<void> {
  return invoke("open_external", { url });
}

export function cover(url: string): Promise<string | null> {
  return invoke<string | null>("cover", { url });
}

export function proxyPrefix(): Promise<string> {
  return invoke<string>("proxy_prefix");
}

export function setAlwaysOnTop(on: boolean): Promise<void> {
  return invoke("set_always_on_top", { on });
}

export function recentLogs(): Promise<string[]> {
  return invoke<string[]>("recent_logs");
}

export interface NetworkStatus {
  dns: string;
  impersonate: boolean;
  obscura: boolean;
  obscura_path: string | null;
  proxy: string | null;
}

export function networkStatus(): Promise<NetworkStatus> {
  return invoke<NetworkStatus>("network_status");
}
export function setImpersonate(on: boolean): Promise<void> {
  return invoke("set_impersonate", { on });
}
export function setObscura(path: string | null): Promise<void> {
  return invoke("set_obscura", { path });
}
export function setProxy(url: string | null): Promise<void> {
  return invoke("set_proxy", { url });
}

export function seasons(url: string): Promise<number[]> {
  return invoke<number[]>("seasons", { url });
}

export function episodes(url: string, season: number): Promise<Episode[]> {
  return invoke<Episode[]>("episodes", { url, season });
}

export function appSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("app_settings");
}

export interface DesktopConfig {
  download_path: string | null;
  language: string | null;
  max_quality: string | null;
  min_quality: string | null;
  bandwidth_kib: number | null;
  download_delay_ms: number | null;
  native_hls: boolean | null;
  no_ffmpeg: boolean | null;
}

/** Mindestabstand zwischen Download-Starts (ms, 0 = aus). */
export function setDownloadDelay(ms: number): Promise<void> {
  return invoke("set_download_delay", { ms });
}

export interface PlatformInfo {
  os: string;
  arch: string;
}

export function platformInfo(): Promise<PlatformInfo> {
  return invoke<PlatformInfo>("platform_info");
}

/** Öffnet den nativen Ordner-Dialog; gibt den gewählten Pfad zurück oder null. */
export function pickDownloadFolder(): Promise<string | null> {
  return invoke<string | null>("pick_download_folder");
}

export function getConfig(): Promise<DesktopConfig> {
  return invoke<DesktopConfig>("get_config");
}

export function setConfig(cfg: DesktopConfig): Promise<void> {
  return invoke("set_config", { cfg });
}

export function autostartEnabled(): Promise<boolean> {
  return invoke<boolean>("autostart_enabled");
}

export function setAutostart(enabled: boolean): Promise<void> {
  return invoke("set_autostart", { enabled });
}

export interface JobView {
  id: number;
  title: string;
  status: string; // queued | resolving | running | muxing | done | error | cancelled
  done: number;
  total: number;
  bytes: number;
  error: string | null;
  queue_pos: number | null; // Position in der Warteschlange (0 = als Nächstes)
}

/** Bandbreitenlimit eines einzelnen Downloads (KiB/s, 0 = unbegrenzt). */
export function setDownloadRate(id: number, kib: number): Promise<void> {
  return invoke("set_download_rate", { id, kib });
}

/** Wartenden Download in der Warteschlange verschieben (hoch/runter). */
export function moveDownload(id: number, up: boolean): Promise<void> {
  return invoke("move_download", { id, up });
}

export function startDownload(
  url: string,
  fileName: string,
  title: string,
  lang?: string,
  hoster?: string,
  maxQuality?: string,
): Promise<number> {
  return invoke<number>("start_download", {
    url,
    fileName,
    title,
    lang: lang ?? null,
    hoster: hoster || null,
    maxQuality: maxQuality || null,
  });
}

export function cancelDownload(id: number): Promise<void> {
  return invoke("cancel_download", { id });
}

/** Entfernt fertige/abgebrochene Jobs aus der Download-Liste. */
export function clearFinishedDownloads(): Promise<void> {
  return invoke("clear_finished_downloads");
}

export function pauseDownload(id: number): Promise<void> {
  return invoke("pause_download", { id });
}

export function resumeDownload(id: number): Promise<void> {
  return invoke("resume_download", { id });
}

/** Globale Bandbreitenbegrenzung in KiB/s (0 = unbegrenzt). */
export function setBandwidthLimit(kib: number): Promise<void> {
  return invoke("set_bandwidth_limit", { kib });
}

export function retryDownload(id: number): Promise<number | null> {
  return invoke<number | null>("retry_download", { id });
}

export function listDownloads(): Promise<JobView[]> {
  return invoke<JobView[]>("list_downloads");
}

export function onDownloadUpdate(cb: (job: JobView) => void): Promise<UnlistenFn> {
  return listen<JobView>("download:update", (e) => cb(e.payload));
}

export interface LibraryEntry {
  name: string;
  size_bytes: number;
  modified: number;
  path: string;
}

export function library(): Promise<LibraryEntry[]> {
  return invoke<LibraryEntry[]>("library");
}

export interface ImportResult {
  imported: number;
  skipped: number;
}

/** Kopiert Mediendateien aus einem Ordner in die Bibliothek. */
export function importFromFolder(path: string): Promise<ImportResult> {
  return invoke<ImportResult>("import_from_folder", { path });
}

/** Taskleisten-/Icon-Fortschritt (0–100) oder null zum Ausblenden. */
export function setTaskbarProgress(progress: number | null): Promise<void> {
  return invoke("set_taskbar_progress", { progress });
}

export function deleteLibraryItem(name: string): Promise<void> {
  return invoke("delete_library_item", { name });
}

export function renameLibraryItem(name: string, newName: string): Promise<void> {
  return invoke("rename_library_item", { name, newName });
}

export function openLibraryItem(name: string): Promise<void> {
  return invoke("open_library_item", { name });
}

export function localMediaUrl(name: string): Promise<string> {
  return invoke<string>("local_media_url", { name });
}

export function revealLibraryItem(name: string): Promise<void> {
  return invoke("reveal_library_item", { name });
}

/** Lokale Datei → asset://-URL fürs <video>-Element. */
export function localFileUrl(path: string): string {
  return convertFileSrc(path);
}
