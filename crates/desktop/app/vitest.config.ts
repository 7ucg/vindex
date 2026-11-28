import { defineConfig } from "vitest/config";
import solid from "vite-plugin-solid";

// Vitest-Setup für das SolidJS-Frontend. jsdom als DOM, damit Komponenten (z. B. ein
// Smoke-Mount von PortalView) laufen; die Dev-Export-Conditions sind für Solids
// Reaktivität nötig (sonst doppelte Effekte). Reine Logik-Tests (sanitize/vgrid/i18n)
// brauchen davon nichts, laufen aber im selben Harness.
export default defineConfig({
  plugins: [solid()],
  resolve: {
    conditions: ["development", "browser"],
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },
});
