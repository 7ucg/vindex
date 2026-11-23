// Inkognito-Sitzung: wenn an, wird die Aktivität dieser Sitzung nicht mitgeschrieben
// (kein Fortschritt/„Weiterschauen", kein Download-Verlauf). Bewusst nicht persistiert —
// der Modus gilt nur bis zum nächsten Start. Explizite Aktionen (Merken) bleiben erlaubt.
import { createSignal } from "solid-js";

const [incognito, setIncognitoSig] = createSignal(false);

export { incognito };
export function setIncognito(on: boolean): void {
  setIncognitoSig(on);
}
export function isIncognito(): boolean {
  return incognito();
}
