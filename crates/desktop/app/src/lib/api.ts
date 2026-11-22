// Typisierte Brücke zu den Tauri-Commands (crates/desktop/app/src-tauri/src/lib.rs).
// Jede Funktion mappt 1:1 auf ein #[tauri::command]; das Frontend ruft nie direkt
// ins Netz, sondern immer über diese Schicht in den Rust-Kern.
import { invoke } from "@tauri-apps/api/core";

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

export function appSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("app_settings");
}
