import { describe, it, expect, vi, beforeEach } from "vitest";
import { render } from "@solidjs/testing-library";

// Mount-Rauchtest: rendert PortalView in jsdom und prüft nur, dass nichts beim Aufbau
// wirft. Fängt genau die Klasse Fehler, die tsc/vite NICHT sehen — TDZ/Initialisierungs-
// reihenfolge in Memos/Effekten (siehe der „visible before initialization"-Crash).
// Die Tauri-API wird gemockt, damit kein echter invoke nötig ist.
vi.mock("../lib/api", () => ({
  discover: vi.fn(() => Promise.resolve([])),
  library: vi.fn(() => Promise.resolve([])),
  seasons: vi.fn(() => Promise.resolve([])),
  episodes: vi.fn(() => Promise.resolve([])),
  startDownload: vi.fn(() => Promise.resolve(0)),
  osLocale: vi.fn(() => Promise.resolve("de")),
}));

import { PortalView } from "./PortalView";

describe("PortalView mount", () => {
  beforeEach(() => {
    try {
      localStorage.clear();
    } catch {
      /* jsdom ohne Storage – egal */
    }
  });

  it("rendert ohne zu werfen (aniworld)", () => {
    const { container, unmount } = render(() => <PortalView portal="aniworld" />);
    expect(container).toBeTruthy();
    expect(container.childElementCount).toBeGreaterThan(0);
    unmount();
  });

  it("rendert ohne zu werfen (filmo)", () => {
    const { unmount } = render(() => <PortalView portal="filmo" />);
    unmount();
  });
});
