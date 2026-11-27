// Übersetzungen für downloads (DE/EN). Schlüssel werden in i18n.ts zusammengeführt.
export const de: Record<string, string> = {
  "downloads.title": "Downloads",
  "downloads.status.queued": "in Warteschlange",
  "downloads.status.resolving": "löst auf …",
  "downloads.status.running": "lädt",
  "downloads.status.paused": "pausiert",
  "downloads.status.muxing": "muxt",
  "downloads.status.done": "fertig",
  "downloads.status.error": "Fehler",
  "downloads.status.cancelled": "abgebrochen",
  "downloads.active": "{n} aktiv",
  "downloads.total": "{n} gesamt",
  "downloads.pauseAll": "Alle pausieren",
  "downloads.resumeAll": "Alle fortsetzen",
  "downloads.cancelAll": "Alle abbrechen",
  "downloads.clearDone": "Erledigte leeren",
  "downloads.gateBanner":
    "Warteschlange pausiert: {reason} — laufende Downloads laufen zu Ende, neue starten erst wieder, wenn die Bedingung erfüllt ist (in den Einstellungen unter „Zeitplan & Abos“).",
  "downloads.empty":
    "Noch keine Downloads. In einer Folge auf „Laden“ tippen — der Fortschritt läuft hier auf.",
  "downloads.etaRemaining": "noch {eta}",
  "downloads.moveUp": "In der Warteschlange nach oben",
  "downloads.moveDown": "In der Warteschlange nach unten",
  "downloads.rateLimitTitle": "Tempolimit nur für diesen Download (KiB/s, 0 = unbegrenzt)",
  "downloads.history": "Verlauf",
  "downloads.clear": "Leeren",
  "downloads.note":
    "Download-Engine, Remux und Naming laufen im Kern. Der Verlauf merkt sich abgeschlossene Downloads lokal. Bandbreitenlimit und Prioritäten folgen.",
};
export const en: Record<string, string> = {
  "downloads.title": "Downloads",
  "downloads.status.queued": "queued",
  "downloads.status.resolving": "resolving …",
  "downloads.status.running": "downloading",
  "downloads.status.paused": "paused",
  "downloads.status.muxing": "muxing",
  "downloads.status.done": "done",
  "downloads.status.error": "error",
  "downloads.status.cancelled": "cancelled",
  "downloads.active": "{n} active",
  "downloads.total": "{n} total",
  "downloads.pauseAll": "Pause all",
  "downloads.resumeAll": "Resume all",
  "downloads.cancelAll": "Cancel all",
  "downloads.clearDone": "Clear finished",
  "downloads.gateBanner":
    "Queue paused: {reason} — running downloads finish, new ones only start again once the condition is met (in Settings under “Schedule & Subscriptions”).",
  "downloads.empty":
    "No downloads yet. Hit “Download” on an episode — the progress shows up here.",
  "downloads.etaRemaining": "{eta} left",
  "downloads.moveUp": "Move up in the queue",
  "downloads.moveDown": "Move down in the queue",
  "downloads.rateLimitTitle": "Rate limit for this download only (KiB/s, 0 = unlimited)",
  "downloads.history": "History",
  "downloads.clear": "Clear",
  "downloads.note":
    "The download engine, remux and naming run in the core. History keeps finished downloads locally. Bandwidth limits and priorities are coming.",
};
