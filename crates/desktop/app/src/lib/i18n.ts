// Sprache (DE/EN) für die Oberfläche. Reaktiv über ein Signal: t() liest locale(),
// also rendern Komponenten bei Sprachwechsel neu. Übersetzungen liegen pro View in
// src/lib/i18n/<view>.ts (je ein de/en-Objekt) und werden hier zusammengeführt —
// so kann an jeder View unabhängig gearbeitet werden, ohne ein zentrales Wörterbuch.
import { createSignal } from "solid-js";
import { osLocale } from "./api";

import { de as appDe, en as appEn } from "./i18n/app";
import { de as sidebarDe, en as sidebarEn } from "./i18n/sidebar";
import { de as homeDe, en as homeEn } from "./i18n/home";
import { de as detailDe, en as detailEn } from "./i18n/detail";
import { de as downloadsDe, en as downloadsEn } from "./i18n/downloads";
import { de as libraryDe, en as libraryEn } from "./i18n/library";
import { de as playerDe, en as playerEn } from "./i18n/player";
import { de as portalDe, en as portalEn } from "./i18n/portal";
import { de as settingsDe, en as settingsEn } from "./i18n/settings";
import { de as paletteDe, en as paletteEn } from "./i18n/palette";
import { de as commonDe, en as commonEn } from "./i18n/common";

export type Lang = "de" | "en";
type Dict = Record<string, string>;

const DE: Dict = {
  ...commonDe,
  ...appDe,
  ...sidebarDe,
  ...homeDe,
  ...detailDe,
  ...downloadsDe,
  ...libraryDe,
  ...playerDe,
  ...portalDe,
  ...settingsDe,
  ...paletteDe,
};
const EN: Dict = {
  ...commonEn,
  ...appEn,
  ...sidebarEn,
  ...homeEn,
  ...detailEn,
  ...downloadsEn,
  ...libraryEn,
  ...playerEn,
  ...portalEn,
  ...settingsEn,
  ...paletteEn,
};
const DICTS: Record<Lang, Dict> = { de: DE, en: EN };

const KEY = "vindex:lang";

function stored(): Lang | null {
  try {
    const v = localStorage.getItem(KEY);
    return v === "de" || v === "en" ? v : null;
  } catch {
    return null;
  }
}

const [lang, setLangRaw] = createSignal<Lang>(stored() ?? "de");
export { lang };

export function setLang(l: Lang): void {
  setLangRaw(l);
  try {
    localStorage.setItem(KEY, l);
  } catch {
    /* localStorage blockiert — egal, bleibt für diese Sitzung gesetzt */
  }
}

// Übersetzen. Fehlt der Schlüssel in der aktiven Sprache, Fallback auf Deutsch,
// sonst der Schlüssel selbst (fällt im UI auf, statt leer zu sein). Platzhalter
// {name} werden aus params ersetzt.
export function t(key: string, params?: Record<string, string | number>): string {
  const d = DICTS[lang()] ?? DE;
  let s = d[key] ?? DE[key] ?? key;
  if (params) {
    for (const k in params) s = s.split(`{${k}}`).join(String(params[k]));
  }
  return s;
}

// Ohne gespeicherte Wahl einmalig an die System-Locale angleichen (nur de/en).
let initialized = false;
export function initLangFromSystem(): void {
  if (initialized || stored()) return;
  initialized = true;
  void osLocale()
    .then((loc) => {
      if (loc && loc.toLowerCase().startsWith("en")) setLangRaw("en");
    })
    .catch(() => {});
}
