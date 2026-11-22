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

export function search(keyword: string): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("search", { keyword });
}

export function catalog(portal: string): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("catalog", { portal });
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

export function resolvePlayback(url: string): Promise<PlaybackSource> {
  return invoke<PlaybackSource>("resolve_playback", { url });
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
}

export function titleMeta(url: string): Promise<TitleMeta> {
  return invoke<TitleMeta>("title_meta", { url });
}

export function cover(url: string): Promise<string | null> {
  return invoke<string | null>("cover", { url });
}

export function proxyPrefix(): Promise<string> {
  return invoke<string>("proxy_prefix");
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

export interface JobView {
  id: number;
  title: string;
  status: string; // queued | resolving | running | muxing | done | error | cancelled
  done: number;
  total: number;
  bytes: number;
  error: string | null;
}

export function startDownload(url: string, fileName: string, title: string): Promise<number> {
  return invoke<number>("start_download", { url, fileName, title });
}

export function cancelDownload(id: number): Promise<void> {
  return invoke("cancel_download", { id });
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

export function deleteLibraryItem(name: string): Promise<void> {
  return invoke("delete_library_item", { name });
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
