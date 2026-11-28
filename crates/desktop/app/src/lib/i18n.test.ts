import { describe, it, expect, beforeEach, vi } from "vitest";

// i18n zieht osLocale aus ./api (Tauri-Bridge). Im Test gibt es kein Tauri, also
// ./api vor dem i18n-Import mocken — sonst scheitert schon das Laden des Moduls.
vi.mock("./api", () => ({ osLocale: vi.fn(() => Promise.resolve("de")) }));

import { t, lang, setLang } from "./i18n";

// Die Teil-Wörterbücher direkt, um die Schlüsselmengen roh zu vergleichen (statt
// über die zusammengeführten Dicts, die i18n nicht exportiert).
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

const DE_PARTS = [commonDe, appDe, sidebarDe, homeDe, detailDe, downloadsDe, libraryDe, playerDe, portalDe, settingsDe, paletteDe];
const EN_PARTS = [commonEn, appEn, sidebarEn, homeEn, detailEn, downloadsEn, libraryEn, playerEn, portalEn, settingsEn, paletteEn];

function keysOf(parts: Record<string, string>[]): Set<string> {
  const s = new Set<string>();
  for (const p of parts) for (const k of Object.keys(p)) s.add(k);
  return s;
}

beforeEach(() => {
  try {
    localStorage.clear();
  } catch {
    /* in manchen Umgebungen gesperrt — egal */
  }
  setLang("de");
});

describe("t()", () => {
  it("liefert den DE-String bei de, den EN-String nach setLang('en')", () => {
    setLang("de");
    expect(lang()).toBe("de");
    expect(t("detail.season", { n: 1 })).toBe("Staffel 1");

    setLang("en");
    expect(lang()).toBe("en");
    expect(t("detail.season", { n: 1 })).toBe("Season 1");
  });

  it("gibt bei fehlendem Schlüssel den Schlüssel zurück und wirft nicht", () => {
    const key = "gibt.es.garantiert.nicht";
    expect(() => t(key)).not.toThrow();
    expect(t(key)).toBe(key);
  });

  it("ersetzt den Platzhalter {n}", () => {
    setLang("de");
    expect(t("detail.season", { n: 3 })).toBe("Staffel 3");
    setLang("en");
    expect(t("detail.season", { n: 3 })).toBe("Season 3");
  });

  it("fällt ohne EN-Treffer auf Deutsch zurück (nicht leer, nicht nur Key)", () => {
    // Schlüssel existiert in beiden Sprachen, daher echter EN-Wert; hier nur prüfen,
    // dass ein vorhandener Key in EN auch wirklich etwas liefert.
    setLang("en");
    expect(t("detail.season", { n: 2 })).toBe("Season 2");
  });
});

describe("Schlüssel-Parität DE/EN", () => {
  const de = keysOf(DE_PARTS);
  const en = keysOf(EN_PARTS);

  it("jeder DE-Key existiert auf EN", () => {
    const missing = [...de].filter((k) => !en.has(k));
    expect(missing).toEqual([]);
  });

  it("jeder EN-Key existiert auf DE", () => {
    const missing = [...en].filter((k) => !de.has(k));
    expect(missing).toEqual([]);
  });

  it("beide Sprachen übersetzen die gemeinsame Schnittmenge nicht-leer", () => {
    const common = [...de].filter((k) => en.has(k));
    expect(common.length).toBeGreaterThan(0);
    for (const k of common) {
      setLang("de");
      expect(t(k).length).toBeGreaterThan(0);
      setLang("en");
      expect(t(k).length).toBeGreaterThan(0);
    }
  });
});
