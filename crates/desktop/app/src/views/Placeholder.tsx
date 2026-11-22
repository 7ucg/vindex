import type { JSX } from "solid-js";

// Vorläufige Ansicht für noch nicht gebaute Bereiche. Zeigt, was hier in welcher Phase kommt.
export function Placeholder(props: { title: string; phase: string; desc: string }): JSX.Element {
  return (
    <div class="wrap">
      <div class="row-h">
        <h2>{props.title}</h2>
        <span class="link">{props.phase}</span>
      </div>
      <div class="empty">{props.desc}</div>
    </div>
  );
}
