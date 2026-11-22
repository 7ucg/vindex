import type { JSX } from "solid-js";

// Das vindex-Mark: V-Chevron (Monogramm + Download-Pfeil) mit Play-Dreieck in der Kehle.
// Gradient-IDs einmalig, damit mehrere Instanzen sich nicht überschreiben.
export function Logo(props: { class?: string }): JSX.Element {
  return (
    <svg class={props.class} viewBox="0 0 512 512" aria-label="vindex">
      <defs>
        <linearGradient id="vx-bg" x1="0" y1="0" x2="512" y2="512" gradientUnits="userSpaceOnUse">
          <stop offset="0" stop-color="#1b1030" />
          <stop offset="1" stop-color="#0c0a16" />
        </linearGradient>
        <linearGradient id="vx-v" x1="128" y1="120" x2="384" y2="400" gradientUnits="userSpaceOnUse">
          <stop offset="0" stop-color="#7c5cff" />
          <stop offset="0.55" stop-color="#b14cff" />
          <stop offset="1" stop-color="#ff4d9d" />
        </linearGradient>
        <linearGradient id="vx-p" x1="210" y1="200" x2="320" y2="320" gradientUnits="userSpaceOnUse">
          <stop offset="0" stop-color="#9affef" />
          <stop offset="1" stop-color="#36d1ff" />
        </linearGradient>
      </defs>
      <rect x="24" y="24" width="464" height="464" rx="112" fill="url(#vx-bg)" />
      <rect
        x="24.5"
        y="24.5"
        width="463"
        height="463"
        rx="111.5"
        fill="none"
        stroke="#ffffff"
        stroke-opacity="0.08"
      />
      <path d="M132 136 L256 392 L380 136 L324 136 L256 274 L188 136 Z" fill="url(#vx-v)" />
      <path d="M226 212 L226 300 L300 256 Z" fill="url(#vx-p)" />
      <path
        d="M256 300 L256 356"
        stroke="#ffffff"
        stroke-opacity="0.22"
        stroke-width="10"
        stroke-linecap="round"
      />
    </svg>
  );
}
