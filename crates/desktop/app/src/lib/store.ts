// Globaler UI-Zustand (bewusst minimal): aktive Ansicht + aktives Portal.
import { createSignal } from "solid-js";

export type View =
  | "home"
  | "library"
  | "downloads"
  | "aniworld"
  | "serienstream"
  | "filmo"
  | "settings";

export type Portal = "aniworld" | "serienstream" | "filmo";

export const PORTALS: { id: Portal; label: string; accent: string }[] = [
  { id: "aniworld", label: "aniworld.to", accent: "var(--v2)" },
  { id: "serienstream", label: "serienstream.to", accent: "var(--v1)" },
  { id: "filmo", label: "filmo.to", accent: "var(--play)" },
];

const [view, setView] = createSignal<View>("home");

export { view, setView };
