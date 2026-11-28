import { describe, it, expect } from "vitest";
import { sanitizeFilename, RESERVED_NAMES } from "./sanitize";

// Muss deckungsgleich mit Rust sanitize_title_component bleiben. Weicht eine Seite
// ab, divergieren Download-Name und Abo-Abgleich -> Endlos-Re-Download. Darum hier
// jede Regel einzeln plus die Reihenfolge-Faelle festgenagelt.

const cp = (s: string) => Array.from(s).length;

describe("sanitizeFilename", () => {
  // --- Stub-Faelle, bleiben ---
  it("entfernt FS-verbotene Zeichen", () => {
    expect(sanitizeFilename('a/b:c*d?"e<f>g|h')).toBe("abcdefgh");
  });
  it("leerer Rest wird zu 'titel'", () => {
    expect(sanitizeFilename("///")).toBe("titel");
  });

  // --- Regel 1: Steuerzeichen (Cc) -> Leerzeichen ---
  it("Steuerzeichen werden zu Leerzeichen, nicht entfernt", () => {
    expect(sanitizeFilename("a\u0000b")).toBe("a b");
    expect(sanitizeFilename("a\tb")).toBe("a b");
    expect(sanitizeFilename("a\nb")).toBe("a b");
    // U+007F und C1-Bereich (U+0080..U+009F) sind ebenfalls Cc.
    expect(sanitizeFilename("a\u007fb")).toBe("a b");
    expect(sanitizeFilename("a\u0085b")).toBe("a b");
  });
  it("nur Steuerzeichen -> getrimmt leer -> 'titel'", () => {
    expect(sanitizeFilename("\u0000\u0001\u0002")).toBe("titel");
  });

  // --- Regel 2: \ / : * ? " < > | entfernen ---
  it("entfernt jedes einzelne verbotene Zeichen", () => {
    for (const ch of ["\\", "/", ":", "*", "?", '"', "<", ">", "|"]) {
      expect(sanitizeFilename(`x${ch}y`)).toBe("xy");
    }
  });
  it("verbotene Zeichen werden entfernt, nicht durch Leerzeichen ersetzt", () => {
    expect(sanitizeFilename("a|b")).toBe("ab");
  });
  it("nur verbotene Zeichen -> 'titel'", () => {
    expect(sanitizeFilename('\\/:*?"<>|')).toBe("titel");
  });

  // --- Regel 3: ZUERST auf 120 Codepoints kuerzen ---
  it("kuerzt auf 120, laesst 120 unveraendert", () => {
    expect(sanitizeFilename("a".repeat(120))).toBe("a".repeat(120));
    expect(sanitizeFilename("a".repeat(121))).toBe("a".repeat(120));
    expect(cp(sanitizeFilename("a".repeat(500)))).toBe(120);
  });

  // --- Regel 3 vor Regel 4: Kuerzen passiert vor dem Normalisieren ---
  it("kuerzt vor dem Entfernen fuehrender Punkte (Reihenfolge)", () => {
    // Punkt + 120 'a': nach Kuerzen auf 120 -> Punkt + 119 'a', dann fuehrenden
    // Punkt weg -> 119 'a'. Bei Normalisieren-zuerst waeren es 120.
    expect(sanitizeFilename("." + "a".repeat(120))).toBe("a".repeat(119));
  });
  it("Zeichen 121 ist ein Punkt -> faellt durch Kuerzen weg, 120 bleiben", () => {
    expect(sanitizeFilename("a".repeat(120) + ".")).toBe("a".repeat(120));
  });
  it("Punkt auf Position 120 wird nach Kuerzen weggetrimmt", () => {
    // 119 'a' + Punkt = 120 Zeichen, alle behalten, dann Punkt-Trim -> 119.
    expect(sanitizeFilename("a".repeat(119) + ".")).toBe("a".repeat(119));
  });
  it("Zeichen 121 ist ein Space -> faellt durch Kuerzen weg", () => {
    expect(sanitizeFilename("a".repeat(120) + " ")).toBe("a".repeat(120));
  });

  // --- Regel 4: trim, Punkte raus, trim ---
  it("trimmt Rand-Whitespace", () => {
    expect(sanitizeFilename("  name  ")).toBe("name");
  });
  it("entfernt fuehrende und abschliessende Punkte", () => {
    expect(sanitizeFilename(".....name.....")).toBe("name");
  });
  it("Punkte in der Mitte bleiben", () => {
    expect(sanitizeFilename("a.b.c")).toBe("a.b.c");
  });
  it("zweites trim greift: Space zwischen Punkt und Rand", () => {
    // trim -> '.name.' ; Punkte weg -> ' name ' ; zweites trim -> 'name'
    expect(sanitizeFilename(" . name . ")).toBe("name");
  });
  it("nur Punkte -> 'titel'", () => {
    expect(sanitizeFilename("....")).toBe("titel");
  });
  it("nur Whitespace -> 'titel'", () => {
    expect(sanitizeFilename("     ")).toBe("titel");
  });

  // --- Regel 5: reservierte Windows-Namen, case-insensitiv -> '_' anhaengen ---
  it("haengt '_' an jeden reservierten Namen (Original-Schreibweise)", () => {
    for (const name of RESERVED_NAMES) {
      expect(sanitizeFilename(name)).toBe(name + "_");
    }
  });
  it("reserviert ist case-insensitiv", () => {
    expect(sanitizeFilename("con")).toBe("con_");
    expect(sanitizeFilename("Nul")).toBe("Nul_");
    expect(sanitizeFilename("com1")).toBe("com1_");
    expect(sanitizeFilename("lpt9")).toBe("lpt9_");
  });
  it("nur der exakte Name ist reserviert", () => {
    expect(sanitizeFilename("CONtent")).toBe("CONtent");
    expect(sanitizeFilename("con1")).toBe("con1");
    expect(sanitizeFilename("ACON")).toBe("ACON");
    expect(sanitizeFilename("COM0")).toBe("COM0");
    expect(sanitizeFilename("COM10")).toBe("COM10");
    expect(sanitizeFilename("LPT")).toBe("LPT");
  });
  it("reserviert erst nach Normalisieren (Punkte/Space)", () => {
    expect(sanitizeFilename("  con.  ")).toBe("con_");
    // Verbotene Zeichen weg -> 'CON' -> reserviert.
    expect(sanitizeFilename("C/O/N")).toBe("CON_");
  });

  // --- Regel 6: leeres Ergebnis -> 'titel' ---
  it("leere Eingabe -> 'titel'", () => {
    expect(sanitizeFilename("")).toBe("titel");
  });
  it("'titel' ist nicht reserviert und bleibt", () => {
    expect(sanitizeFilename("titel")).toBe("titel");
  });

  // --- Unicode / Codepoints (Rust char == JS Codepoint) ---
  it("zaehlt Astralzeichen als einen Codepoint an der 120-Grenze", () => {
    const out = sanitizeFilename("\u{1F600}".repeat(120) + "x");
    expect(out).toBe("\u{1F600}".repeat(120));
    expect(cp(out)).toBe(120);
  });
  it("121 Emojis werden auf 120 gekuerzt", () => {
    expect(cp(sanitizeFilename("\u{1F600}".repeat(121)))).toBe(120);
  });
  it("Unicode-Buchstaben bleiben erhalten", () => {
    expect(sanitizeFilename("Müller — Straße ©")).toBe("Müller — Straße ©");
  });

  // --- Idempotenz ---
  it("ist idempotent", () => {
    const inputs = [
      "a/b:c",
      ".....name.....",
      "con",
      "CON",
      "nul.",
      "a".repeat(200),
      "." + "a".repeat(120),
      "\u{1F600}".repeat(130),
      "   ",
      "....",
      "",
      '\\/:*?"<>|',
      "CONtent",
      "a\u0000b",
      "titel",
    ];
    for (const s of inputs) {
      const once = sanitizeFilename(s);
      expect(sanitizeFilename(once)).toBe(once);
    }
  });
});
