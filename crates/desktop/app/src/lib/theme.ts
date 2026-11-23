// Theme, Akzentfarbe, Dichte und Darstellungsgröße — im WebView-localStorage persistiert.
export type Theme = "dark" | "light" | "amoled";
export type Accent = "violett" | "cyan" | "gruen" | "pink" | "bernstein";
export type Density = "normal" | "kompakt";
export type Scale = "90" | "100" | "110" | "125";

const THEME_KEY = "vindex:theme";
const ACCENT_KEY = "vindex:accent";
const DENSITY_KEY = "vindex:density";
const SCALE_KEY = "vindex:scale";

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
const SCALES: Scale[] = ["90", "100", "110", "125"];

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

export function getScale(): Scale {
  return read(SCALE_KEY, SCALES, "100");
}
export function applyScale(s: Scale): void {
  // zoom skaliert das gesamte Dokument gleichmäßig (WebKitGTK-tauglich, inkl. fixe Overlays).
  (document.documentElement.style as CSSStyleDeclaration & { zoom?: string }).zoom = `${
    parseInt(s, 10) / 100
  }`;
  write(SCALE_KEY, s);
}

/** Beim Start alles anwenden. */
export function applyAll(): void {
  applyTheme(getTheme());
  applyAccent(getAccent());
  applyDensity(getDensity());
  applyScale(getScale());
}
