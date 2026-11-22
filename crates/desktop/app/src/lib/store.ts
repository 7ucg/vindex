// Globaler UI-Zustand (bewusst minimal): aktive Ansicht, aktives Portal, gewählter Titel.
import { createSignal } from "solid-js";
import type { Episode } from "./api";

export type View =
  | "home"
  | "library"
  | "downloads"
  | "aniworld"
  | "serienstream"
  | "filmo"
  | "detail"
  | "settings";

export type Portal = "aniworld" | "serienstream" | "filmo";

/** Ein zum Ansehen ausgewählter Titel (Quelle fürs Zurück-Navigieren). */
export interface Selected {
  title: string;
  url: string;
  from: View;
}

export const PORTALS: { id: Portal; label: string; accent: string }[] = [
  { id: "aniworld", label: "aniworld.to", accent: "var(--v2)" },
  { id: "serienstream", label: "serienstream.to", accent: "var(--v1)" },
  { id: "filmo", label: "filmo.to", accent: "var(--play)" },
];

/** Folgen-Kontext fürs automatische Weiterspielen. */
export interface PlayQueue {
  showTitle: string;
  episodes: Episode[];
  index: number;
}

/** Ein laufender Player-Stream (proxied URL + HLS-Flag + Anzeigetitel + Queue). */
export interface Playback {
  title: string;
  url: string;
  isHls: boolean;
  queue?: PlayQueue;
}

const [view, setView] = createSignal<View>("home");
const [selected, setSelected] = createSignal<Selected | null>(null);
const [playback, setPlayback] = createSignal<Playback | null>(null);

export function openPlayer(p: Playback): void {
  setPlayback(p);
}
export function closePlayer(): void {
  setPlayback(null);
}

/** Öffnet die Detailansicht für einen Titel und merkt sich, woher man kam. */
export function openTitle(title: string, url: string): void {
  setSelected({ title, url, from: view() });
  setView("detail");
}

export { view, setView, selected, setSelected, playback };
