// Merkliste (Watchlist) im WebView-localStorage. Rein lokal pro Gerät.
export interface WatchItem {
  url: string;
  title: string;
}

const KEY = "vindex:watchlist";

function read(): WatchItem[] {
  try {
    return JSON.parse(localStorage.getItem(KEY) || "[]") as WatchItem[];
  } catch {
    return [];
  }
}
function write(l: WatchItem[]): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(l));
  } catch {
    /* blockiert — egal */
  }
}

export function watchlist(): WatchItem[] {
  return read();
}

export function inWatchlist(url: string): boolean {
  return read().some((x) => x.url === url);
}

/** Legt an oder entfernt; gibt den neuen Zustand zurück (true = jetzt gemerkt). */
export function toggleWatchlist(item: WatchItem): boolean {
  const l = read();
  const i = l.findIndex((x) => x.url === item.url);
  if (i >= 0) {
    l.splice(i, 1);
    write(l);
    return false;
  }
  l.unshift(item);
  write(l);
  return true;
}
