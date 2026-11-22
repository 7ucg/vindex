// Poster-Cover fürs Grid: lazy pro Titel, gecacht und mit Concurrency-Limit,
// damit das Portal nicht geflutet wird. Die rohe Cover-URL wird gecacht; die
// Proxy-URL (CSP erlaubt Bilder nur von 127.0.0.1) wird im Frontend gebaut.
import { cover as apiCover, proxyPrefix as apiPrefix } from "./api";

const RAW_KEY = "vindex:covers";
const MAX_INFLIGHT = 3;

// rohe Cover-URL pro Titel-URL ("" = kein Cover gefunden).
const raw: Map<string, string> = load();

function load(): Map<string, string> {
  try {
    return new Map(Object.entries(JSON.parse(localStorage.getItem(RAW_KEY) || "{}")));
  } catch {
    return new Map();
  }
}
let persistTimer: number | undefined;
function persist(): void {
  clearTimeout(persistTimer);
  persistTimer = window.setTimeout(() => {
    try {
      localStorage.setItem(RAW_KEY, JSON.stringify(Object.fromEntries(raw)));
    } catch {
      /* egal */
    }
  }, 400);
}

let prefixPromise: Promise<string> | null = null;
function prefix(): Promise<string> {
  return (prefixPromise ??= apiPrefix());
}

function b64url(s: string): string {
  return btoa(unescape(encodeURIComponent(s)))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

let active = 0;
const waiting: (() => void)[] = [];
function acquire(): Promise<void> {
  if (active < MAX_INFLIGHT) {
    active++;
    return Promise.resolve();
  }
  return new Promise<void>((res) => waiting.push(() => res())).then(() => {
    active++;
  });
}
function release(): void {
  active--;
  waiting.shift()?.();
}

/** Liefert die proxied Cover-URL fürs `<img>` oder null (kein Cover). */
export async function coverUrl(titleUrl: string): Promise<string | null> {
  let r = raw.get(titleUrl);
  if (r === undefined) {
    await acquire();
    try {
      r = (await apiCover(titleUrl)) ?? "";
    } catch {
      r = "";
    } finally {
      release();
    }
    raw.set(titleUrl, r);
    persist();
  }
  if (!r) return null;
  return (await prefix()) + b64url(r);
}
