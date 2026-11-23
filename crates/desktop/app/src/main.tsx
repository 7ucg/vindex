/* @refresh reload */
import { render } from "solid-js/web";
import App from "./App";
import "./styles.css";
import { applyAll } from "./lib/theme";
import { platformInfo } from "./lib/api";

applyAll();

// Plattform erkennen und als data-Attribute setzen, damit sich die Oberfläche
// anpassen kann (macOS-Ampel-Abstand, Windows/Linux, oder reiner Web-Betrieb ohne
// Tauri). In einer normalen Web-Umgebung schlägt der Command fehl → data-os="web".
function detectPlatform(): void {
  const el = document.documentElement;
  const hasTauri = typeof (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !== "undefined";
  el.setAttribute("data-runtime", hasTauri ? "desktop" : "web");
  if (!hasTauri) {
    el.setAttribute("data-os", "web");
    return;
  }
  platformInfo()
    .then((p) => el.setAttribute("data-os", p.os))
    .catch(() => el.setAttribute("data-os", "web"));
}
detectPlatform();

const root = document.getElementById("root");
if (root) render(() => <App />, root);
