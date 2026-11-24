// Umbelegbare Player-Tastenkürzel. Die Pfeiltasten (±5/±30 s), Ziffern (0–9
// springen) und Escape bleiben fest; hier stehen nur die Einzeltasten-Aktionen.
// Overrides liegen lokal in localStorage.

export type Action =
  | "playpause"
  | "back10"
  | "fwd10"
  | "mute"
  | "fullscreen"
  | "next"
  | "skip"
  | "pip"
  | "speeddown"
  | "speedup"
  | "frameback"
  | "framefwd"
  | "loop"
  | "subback"
  | "subfwd"
  | "fit";

export const ACTION_LABELS: Record<Action, string> = {
  playpause: "Play / Pause",
  back10: "10 s zurück",
  fwd10: "10 s vor",
  mute: "Stumm",
  fullscreen: "Vollbild",
  next: "Nächste Folge",
  skip: "Intro/Outro überspringen",
  pip: "Bild-in-Bild",
  speeddown: "Langsamer",
  speedup: "Schneller",
  frameback: "Ein Bild zurück",
  framefwd: "Ein Bild vor",
  loop: "A-B-Loop",
  subback: "Untertitel früher",
  subfwd: "Untertitel später",
  fit: "Zuschnitt",
};

const DEFAULTS: Record<Action, string> = {
  playpause: " ",
  back10: "j",
  fwd10: "l",
  mute: "m",
  fullscreen: "f",
  next: "n",
  skip: "s",
  pip: "p",
  speeddown: "[",
  speedup: "]",
  frameback: ",",
  framefwd: ".",
  loop: "a",
  subback: "g",
  subfwd: "h",
  fit: "z",
};

export const ACTIONS = Object.keys(DEFAULTS) as Action[];

const KEY = "vindex:keybindings";

// Einzelzeichen klein schreiben, damit Groß/Klein nicht auseinanderläuft; Sondertasten bleiben.
function norm(key: string): string {
  return key.length === 1 ? key.toLowerCase() : key;
}

function load(): Partial<Record<Action, string>> {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) || "{}");
    return v && typeof v === "object" ? (v as Partial<Record<Action, string>>) : {};
  } catch {
    return {};
  }
}

let overrides: Partial<Record<Action, string>> = load();

function save(): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(overrides));
  } catch {
    /* egal */
  }
}

/** Aktuelle Belegung (Defaults plus Overrides). */
export function bindings(): Record<Action, string> {
  return { ...DEFAULTS, ...overrides };
}

/** Taste aus einem Keydown-Event auf eine Aktion abbilden (oder null). */
export function actionForKey(key: string): Action | null {
  const k = norm(key);
  const b = bindings();
  for (const a of ACTIONS) {
    if (norm(b[a]) === k) return a;
  }
  return null;
}

/** Eine Aktion neu belegen. Dieselbe Taste wird von anderen Aktionen entfernt. */
export function setBinding(action: Action, key: string): void {
  const k = norm(key);
  const next: Partial<Record<Action, string>> = { ...overrides };
  // Kollision auflösen: steht die Taste schon woanders, dort entfernen (Default übernimmt).
  for (const a of ACTIONS) {
    if (a !== action && norm(bindings()[a]) === k) next[a] = "";
  }
  next[action] = k;
  overrides = next;
  save();
}

/** Alle Overrides löschen (zurück auf Defaults). */
export function resetBindings(): void {
  overrides = {};
  save();
}
