import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { coverUrl } from "../lib/covers";

// Deterministische Platzhalter-Fläche aus dem Titel, bis das echte Cover geladen ist.
function hue(title: string): number {
  let h = 0;
  for (let i = 0; i < title.length; i++) h = (h * 31 + title.charCodeAt(i)) % 360;
  return h;
}
function placeholder(title: string): string {
  const a = hue(title);
  const b = (a + 40) % 360;
  return `radial-gradient(120% 120% at 30% 10%, hsl(${a} 45% 32%), hsl(${b} 55% 12%))`;
}

// Lazy-Poster: holt das Cover erst, wenn die Karte (fast) sichtbar ist.
export function Cover(props: { url: string; title: string }): JSX.Element {
  const [src, setSrc] = createSignal<string | null>(null);
  let el: HTMLDivElement | undefined;

  onMount(() => {
    const io = new IntersectionObserver(
      (entries) => {
        if (entries[0]?.isIntersecting) {
          io.disconnect();
          coverUrl(props.url)
            .then(setSrc)
            .catch(() => {});
        }
      },
      { rootMargin: "300px" },
    );
    if (el) io.observe(el);
    onCleanup(() => io.disconnect());
  });

  return (
    <div
      ref={el}
      class="ph"
      style={src() ? { background: "none" } : { background: placeholder(props.title) }}
    >
      <Show when={src()}>
        <img src={src()!} alt={props.title} loading="lazy" onError={() => setSrc(null)} />
      </Show>
    </div>
  );
}
