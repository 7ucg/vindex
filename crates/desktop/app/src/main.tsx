/* @refresh reload */
import { render } from "solid-js/web";
import App from "./App";
import "./styles.css";
import { applyAll } from "./lib/theme";
import { platformInfo } from "./lib/api";

applyAll();

// Grobe OS-Schätzung aus dem User-Agent — synchron, damit die plattformabhängigen
// Styles schon beim ersten Paint stimmen (kein Flash). Tauri liefert gleich den
// verlässlichen Wert nach.
function guessOs(): string {
  const ua = navigator.userAgent;
  if (/Mac/i.test(ua)) return "macos";
  if (/Win/i.test(ua)) return "windows";
  if (/Android/i.test(ua)) return "android";
  if (/Linux|X11/i.test(ua)) return "linux";
  return "web";
}

// Plattform erkennen und als data-Attribute setzen, damit sich die Oberfläche
// anpassen kann (macOS-Abstände, Windows/Linux, oder reiner Web-Betrieb ohne Tauri).
function detectPlatform(): void {
  const el = document.documentElement;
  const hasTauri = typeof (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !== "undefined";
  el.setAttribute("data-runtime", hasTauri ? "desktop" : "web");
  // Erst die synchrone Schätzung, damit das erste Bild schon passt.
  el.setAttribute("data-os", hasTauri ? guessOs() : "web");
  if (!hasTauri) return;
  // Dann der verlässliche Wert aus dem Kern (korrigiert die Schätzung, falls nötig).
  platformInfo()
    .then((p) => el.setAttribute("data-os", p.os))
    .catch(() => {
      /* bleibt bei der Schätzung */
    });
}
detectPlatform();

const root = document.getElementById("root");
if (root) {
  render(() => <App />, root);
  // Kaltstart messen: Zeit bis das erste Bild steht (ms seit Seitenstart).
  requestAnimationFrame(() =>
    requestAnimationFrame(() => console.info(`UI bereit ${Math.round(performance.now())} ms`)),
  );
}
