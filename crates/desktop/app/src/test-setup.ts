import { afterEach } from "vitest";
import { cleanup } from "@solidjs/testing-library";

// Nach jedem Test das gerenderte DOM abräumen, damit Mounts sich nicht überlagern.
afterEach(() => cleanup());

// jsdom kennt diese Observer nicht; PortalView (ResizeObserver) und Cover
// (IntersectionObserver, Lazy-Load) nutzen sie beim Mount. No-op-Stubs reichen für
// Rauchtests — sie lösen nie aus, die Komponente bleibt beim Fallback-Zustand.
class NoopObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}
const g = globalThis as Record<string, unknown>;
if (typeof g.ResizeObserver === "undefined") g.ResizeObserver = NoopObserver;
if (typeof g.IntersectionObserver === "undefined") g.IntersectionObserver = NoopObserver;

// jsdom implementiert scrollTo nicht (PortalView setzt beim Filterwechsel den Scroll
// zurück). No-op, damit die Rauchtests keine „Not implemented"-Fehler loggen.
if (typeof window !== "undefined") {
  window.scrollTo = () => {};
  Element.prototype.scrollTo = () => {};
}

// jsdom kennt matchMedia nicht; theme.ts fragt es ab. Minimaler Stub (kein Treffer).
if (typeof window !== "undefined" && !window.matchMedia) {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as unknown as MediaQueryList;
}
