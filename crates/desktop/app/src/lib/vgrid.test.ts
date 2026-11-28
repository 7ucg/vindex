import { describe, it, expect } from "vitest";
import { computeWindow } from "./vgrid";

// Virtualisierung: Erwartungswerte von Hand aus der Semantik gerechnet, nicht abgelesen.

describe("computeWindow", () => {
  it("leere Liste gibt alles auf 0", () => {
    const w = computeWindow({
      count: 0,
      cols: 3,
      rowH: 50,
      scrollTop: 400,
      gridTop: 0,
      viewportH: 300,
      overscan: 2,
      fallback: 12,
    });
    expect(w).toEqual({ start: 0, end: 0, pad: 0, tail: 0 });
  });

  it("rowH=0 (noch nicht gemessen) rendert fallback roh", () => {
    const w = computeWindow({
      count: 100,
      cols: 4,
      rowH: 0,
      scrollTop: 0,
      gridTop: 0,
      viewportH: 300,
      overscan: 2,
      fallback: 12,
    });
    // end = min(count, fallback), Rest 0
    expect(w).toEqual({ start: 0, end: 12, pad: 0, tail: 0 });
  });

  it("fallback wird auf count geklemmt", () => {
    const w = computeWindow({
      count: 5,
      cols: 4,
      rowH: 0,
      scrollTop: 0,
      gridTop: 0,
      viewportH: 300,
      overscan: 2,
      fallback: 12,
    });
    expect(w).toEqual({ start: 0, end: 5, pad: 0, tail: 0 });
  });

  it("negatives rowH zaehlt ebenfalls als ungemessen", () => {
    const w = computeWindow({
      count: 40,
      cols: 2,
      rowH: -1,
      scrollTop: 0,
      gridTop: 0,
      viewportH: 300,
      overscan: 1,
      fallback: 8,
    });
    expect(w).toEqual({ start: 0, end: 8, pad: 0, tail: 0 });
  });

  it("cols=0 verhaelt sich wie cols=1", () => {
    const base = {
      count: 10,
      rowH: 50,
      scrollTop: 0,
      gridTop: 0,
      viewportH: 100,
      overscan: 0,
      fallback: 0,
    };
    // rows=10, firstRow=0, startRow=0, rowsInView=ceil(100/50)=2, endRow=2
    const expected = { start: 0, end: 2, pad: 0, tail: 400 };
    expect(computeWindow({ ...base, cols: 0 })).toEqual(expected);
    expect(computeWindow({ ...base, cols: 1 })).toEqual(expected);
  });

  it("Normalfall mit mehreren Spalten: Item-Indizes aus Zeilen", () => {
    // c=3, rows=ceil(50/3)=17, firstRow=floor(250/100)=2,
    // startRow=max(0,2-1)=1, rowsInView=ceil(300/100)=3, endRow=1+3+2=6
    const w = computeWindow({
      count: 50,
      cols: 3,
      rowH: 100,
      scrollTop: 250,
      gridTop: 0,
      viewportH: 300,
      overscan: 1,
      fallback: 0,
    });
    expect(w).toEqual({ start: 3, end: 18, pad: 100, tail: 1100 });
  });

  it("gridTop verschiebt die erste sichtbare Zeile", () => {
    // c=2, rows=20, firstRow=floor((400-100)/60)=5, startRow=max(0,5-1)=4,
    // rowsInView=ceil(180/60)=3, endRow=4+3+2=9
    const w = computeWindow({
      count: 40,
      cols: 2,
      rowH: 60,
      scrollTop: 400,
      gridTop: 100,
      viewportH: 180,
      overscan: 1,
      fallback: 0,
    });
    expect(w).toEqual({ start: 8, end: 18, pad: 240, tail: 660 });
  });

  it("end wird auf count geklemmt (letzte Zeile unvollstaendig)", () => {
    // c=3, rows=ceil(7/3)=3, firstRow=0, startRow=0,
    // rowsInView=ceil(120/40)=3, endRow=min(3,0+3+0)=3, endRow*c=9 -> auf 7
    const w = computeWindow({
      count: 7,
      cols: 3,
      rowH: 40,
      scrollTop: 0,
      gridTop: 0,
      viewportH: 120,
      overscan: 0,
      fallback: 0,
    });
    expect(w).toEqual({ start: 0, end: 7, pad: 0, tail: 0 });
  });

  it("klemmt ans Listenende: grosser scrollTop bleibt in [0,count]", () => {
    // c=1, rows=5, firstRow=floor(10000/50)=200, startRow=min(200,rows-1=4)=4,
    // rowsInView=ceil(200/50)=4, endRow=min(5,4+4)=5
    const w = computeWindow({
      count: 5,
      cols: 1,
      rowH: 50,
      scrollTop: 10000,
      gridTop: 0,
      viewportH: 200,
      overscan: 0,
      fallback: 0,
    });
    expect(w).toEqual({ start: 4, end: 5, pad: 200, tail: 0 });
    // nicht leer, Start zeigt auf die letzte Zeile
    expect(w.start).toBeLessThan(w.end);
    expect(w.start).toBe((Math.ceil(5 / 1) - 1) * 1);
    expect(w.end).toBe(5);
  });

  it("klemmt ans Listenende auch mit mehreren Spalten", () => {
    // c=3, rows=ceil(7/3)=3, firstRow=floor(9999/40)=249,
    // startRow=min(249-2=247, rows-1=2)=2, rowsInView=ceil(120/40)=3, endRow=min(3,2+3+4)=3
    const w = computeWindow({
      count: 7,
      cols: 3,
      rowH: 40,
      scrollTop: 9999,
      gridTop: 0,
      viewportH: 120,
      overscan: 2,
      fallback: 0,
    });
    expect(w).toEqual({ start: 6, end: 7, pad: 80, tail: 0 });
    expect(w.start).toBeLessThan(w.end);
    expect(w.start).toBeLessThanOrEqual(7);
  });

  it("overscan vergroessert das Fenster oben und unten", () => {
    const base = {
      count: 100,
      cols: 2,
      rowH: 50,
      scrollTop: 500,
      gridTop: 0,
      viewportH: 200,
      fallback: 0,
    };
    // overscan=0: rows=50, firstRow=10, startRow=10, rowsInView=4, endRow=14
    const ohne = computeWindow({ ...base, overscan: 0 });
    expect(ohne).toEqual({ start: 20, end: 28, pad: 500, tail: 1800 });
    // overscan=2: startRow=10-2=8, endRow=8+4+4=16
    const mit = computeWindow({ ...base, overscan: 2 });
    expect(mit).toEqual({ start: 16, end: 32, pad: 400, tail: 1700 });
    // oben frueher, unten spaeter
    expect(mit.start).toBeLessThan(ohne.start);
    expect(mit.end).toBeGreaterThan(ohne.end);
  });

  it("firstRow-overscan wird bei 0 geklemmt (kein negativer pad)", () => {
    // c=1, firstRow=floor(50/50)=1, 1-5=-4 -> startRow=0
    const w = computeWindow({
      count: 100,
      cols: 1,
      rowH: 50,
      scrollTop: 50,
      gridTop: 0,
      viewportH: 100,
      overscan: 5,
      fallback: 0,
    });
    expect(w.start).toBe(0);
    expect(w.pad).toBe(0);
  });

  it("pad=startRow*rowH, tail=max(0,(rows-endRow)*rowH), nie negativ", () => {
    const i = {
      count: 50,
      cols: 3,
      rowH: 100,
      scrollTop: 250,
      gridTop: 0,
      viewportH: 300,
      overscan: 1,
      fallback: 0,
    };
    const w = computeWindow(i);
    // c=3, rows=17, startRow=1, endRow=6 (end=18 nicht auf count geklemmt)
    expect(w.pad).toBe(1 * i.rowH);
    expect(w.tail).toBe(Math.max(0, (17 - 6) * i.rowH));
    expect(w.pad).toBeGreaterThanOrEqual(0);
    expect(w.tail).toBeGreaterThanOrEqual(0);
  });
});
