import type { JSX } from "solid-js";

// Vorläufige Ansicht für Bereiche, die noch gebaut werden.
export function Placeholder(props: { title: string; badge?: string; desc: string }): JSX.Element {
  return (
    <div class="wrap">
      <div class="row-h">
        <h2>{props.title}</h2>
        {props.badge && <span class="link">{props.badge}</span>}
      </div>
      <div class="empty">{props.desc}</div>
    </div>
  );
}
