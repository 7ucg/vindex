// Virtualisierung des Portal-Grids: aus den Maßen das sichtbare Fenster (Start/Ende-Index
// plus Spacer-Höhen für eine korrekte Scrollleiste) berechnen. Pure Funktion ohne DOM —
// die Komponente (PortalView) liefert die gemessenen Werte, die Tests die Grenzfälle.

export interface WindowInput {
  /** Anzahl sichtbarer Einträge (nach Filter/Sort). */
  count: number;
  /** Spalten pro Zeile (>= 1 wird erzwungen). */
  cols: number;
  /** Zeilenhöhe in px; <= 0 heißt „noch nicht gemessen". */
  rowH: number;
  /** Aktueller Scroll-Offset des Containers. */
  scrollTop: number;
  /** Oberkante des Grids relativ zum Scroll-Container. */
  gridTop: number;
  /** Sichtbare Höhe des Viewports. */
  viewportH: number;
  /** Vorgerenderte Zeilen über/unter dem Rand. */
  overscan: number;
  /** Roh-Render-Anzahl, solange die Maße fehlen (rowH <= 0). */
  fallback: number;
}

export interface WindowOut {
  /** Erster Item-Index im Fenster. */
  start: number;
  /** Erster Item-Index NACH dem Fenster (exklusiv). */
  end: number;
  /** Platzhalterhöhe oberhalb (px), hält die Scrollposition. */
  pad: number;
  /** Platzhalterhöhe unterhalb (px), hält die Gesamthöhe. */
  tail: number;
}

export function computeWindow(i: WindowInput): WindowOut {
  const n = i.count;
  const c = i.cols || 1;
  const rh = i.rowH;
  if (rh <= 0 || n === 0) {
    return { start: 0, end: Math.min(n, i.fallback), pad: 0, tail: 0 };
  }
  const rows = Math.ceil(n / c);
  const firstRow = Math.floor((i.scrollTop - i.gridTop) / rh);
  // Gegen das Listenende klemmen: sonst steht das Fenster nach dem Verkleinern der
  // Liste (Filter/Sort) jenseits der letzten Zeile und der Bildschirm bleibt leer.
  const startRow = Math.min(Math.max(0, firstRow - i.overscan), Math.max(0, rows - 1));
  const rowsInView = Math.ceil((i.viewportH || 1) / rh);
  const endRow = Math.min(rows, startRow + rowsInView + i.overscan * 2);
  return {
    start: startRow * c,
    end: Math.min(n, endRow * c),
    pad: startRow * rh,
    tail: Math.max(0, (rows - endRow) * rh),
  };
}
