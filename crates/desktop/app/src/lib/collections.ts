// Benutzer-Sammlungen (benannte Listen von Titeln), reaktiv + im localStorage.
// Rein lokal pro Gerät. Reihenfolge: neueste Einträge zuerst.
import { createSignal } from "solid-js";

export interface CollItem {
  url: string;
  title: string;
}
export interface Collection {
  id: string;
  name: string;
  items: CollItem[];
}

const KEY = "vindex:collections";

function load(): Collection[] {
  try {
    const a = JSON.parse(localStorage.getItem(KEY) || "[]");
    return Array.isArray(a) ? (a as Collection[]) : [];
  } catch {
    return [];
  }
}

const [collections, setCollections] = createSignal<Collection[]>(load());
export { collections };

function persist(list: Collection[]): void {
  setCollections(list);
  try {
    localStorage.setItem(KEY, JSON.stringify(list));
  } catch {
    /* blockiert — egal */
  }
}

function uid(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
}

export function createCollection(name: string): string {
  const id = uid();
  persist([...collections(), { id, name: name.trim() || "Sammlung", items: [] }]);
  return id;
}
export function renameCollection(id: string, name: string): void {
  persist(collections().map((c) => (c.id === id ? { ...c, name: name.trim() || c.name } : c)));
}
export function deleteCollection(id: string): void {
  persist(collections().filter((c) => c.id !== id));
}
export function addToCollection(id: string, item: CollItem): void {
  persist(
    collections().map((c) =>
      c.id === id && !c.items.some((x) => x.url === item.url)
        ? { ...c, items: [item, ...c.items] }
        : c,
    ),
  );
}
export function removeFromCollection(id: string, url: string): void {
  persist(
    collections().map((c) =>
      c.id === id ? { ...c, items: c.items.filter((x) => x.url !== url) } : c,
    ),
  );
}
export function inCollection(id: string, url: string): boolean {
  return collections().find((c) => c.id === id)?.items.some((x) => x.url === url) ?? false;
}
