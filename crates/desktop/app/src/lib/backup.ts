// Export/Import des lokalen Zustands (Merkliste, Fortschritt, Erscheinungsbild).
// Rein localStorage — der Cover-Cache bleibt bewusst außen vor (groß, regenerierbar).
const KEYS = [
  "vindex:watchlist",
  "vindex:resume",
  "vindex:recent",
  "vindex:theme",
  "vindex:accent",
  "vindex:density",
];

export function exportState(): void {
  const data: Record<string, unknown> = {};
  for (const k of KEYS) {
    try {
      const v = localStorage.getItem(k);
      if (v !== null) data[k] = JSON.parse(v);
    } catch {
      /* ungültigen Eintrag überspringen */
    }
  }
  const blob = new Blob([JSON.stringify(data, null, 2)], { type: "application/json" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `vindex-backup-${new Date().toISOString().slice(0, 10)}.json`;
  a.click();
  URL.revokeObjectURL(a.href);
}

export async function importState(file: File): Promise<boolean> {
  try {
    const data = JSON.parse(await file.text()) as Record<string, unknown>;
    for (const k of KEYS) {
      if (k in data) {
        try {
          localStorage.setItem(k, JSON.stringify(data[k]));
        } catch {
          /* egal */
        }
      }
    }
    return true;
  } catch {
    return false;
  }
}
