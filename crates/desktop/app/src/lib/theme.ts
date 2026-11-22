// Theme, Akzentfarbe und Dichte — im WebView-localStorage persistiert.
export type Theme = "dark" | "light" | "amoled";
export type Accent = "violett" | "cyan" | "gruen" | "pink" | "bernstein";
export type Density = "normal" | "kompakt";

const THEME_KEY = "vindex:theme";
const ACCENT_KEY = "vindex:accent";
const DENSITY_KEY = "vindex:density";

function read<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    if (v && (allowed as readonly string[]).includes(v)) return v as T;
  } catch {
    /* Standard */
  }
  return fallback;
}
function write(key: string, v: string): void {
  try {
    localStorage.setItem(key, v);
  } catch {
    /* egal */
  }
}

const THEMES: Theme[] = ["dark", "light", "amoled"];
const ACCENTS: Accent[] = ["violett", "cyan", "gruen", "pink", "bernstein"];
const DENSITIES: Density[] = ["normal", "kompakt"];

export function getTheme(): Theme {
  return read(THEME_KEY, THEMES, "dark");
}
export function applyTheme(t: Theme): void {
  document.documentElement.setAttribute("data-theme", t);
  write(THEME_KEY, t);
}

export function getAccent(): Accent {
  return read(ACCENT_KEY, ACCENTS, "violett");
}
export function applyAccent(a: Accent): void {
  document.documentElement.setAttribute("data-accent", a);
  write(ACCENT_KEY, a);
}

export function getDensity(): Density {
  return read(DENSITY_KEY, DENSITIES, "normal");
}
export function applyDensity(d: Density): void {
  document.documentElement.setAttribute("data-density", d);
  write(DENSITY_KEY, d);
}

/** Beim Start alles anwenden. */
export function applyAll(): void {
  applyTheme(getTheme());
  applyAccent(getAccent());
  applyDensity(getDensity());
}
