// Theme-Auswahl, im WebView-localStorage persistiert. Dark ist Standard.
export type Theme = "dark" | "light" | "amoled";

const KEY = "vindex:theme";

export function getTheme(): Theme {
  try {
    const t = localStorage.getItem(KEY);
    if (t === "light" || t === "amoled" || t === "dark") return t;
  } catch {
    /* blockiert — Standard */
  }
  return "dark";
}

export function applyTheme(t: Theme): void {
  document.documentElement.setAttribute("data-theme", t);
  try {
    localStorage.setItem(KEY, t);
  } catch {
    /* egal */
  }
}
