import type { JSX } from "solid-js";

// Kleine Stroke-Icons (18px) für Sidebar und Controls. Bewusst inline, keine Icon-Lib.
const base = {
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  "stroke-width": "2",
  "stroke-linecap": "round" as const,
  "stroke-linejoin": "round" as const,
};

export const IconHome = (): JSX.Element => (
  <svg {...base}>
    <path d="M3 11l9-8 9 8" />
    <path d="M5 9v11h14V9" />
  </svg>
);

export const IconLibrary = (): JSX.Element => (
  <svg {...base}>
    <circle cx="12" cy="12" r="9" />
    <path d="M8 12h8M12 8v8" />
  </svg>
);

export const IconDownload = (): JSX.Element => (
  <svg {...base}>
    <path d="M12 3v12" />
    <path d="M8 11l4 4 4-4" />
    <path d="M4 19h16" />
  </svg>
);

export const IconTv = (): JSX.Element => (
  <svg {...base}>
    <rect x="3" y="4" width="18" height="14" rx="2" />
    <path d="M8 20h8" />
  </svg>
);

export const IconFilm = (): JSX.Element => (
  <svg {...base}>
    <rect x="4" y="4" width="16" height="16" rx="2" />
    <path d="M4 8h16M8 4v16" />
  </svg>
);

export const IconSettings = (): JSX.Element => (
  <svg {...base}>
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.6 1.6 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.6 1.6 0 0 0-2.7 1.1V21a2 2 0 0 1-4 0v-.1A1.6 1.6 0 0 0 7 19.4a1.6 1.6 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.6 1.6 0 0 0-1.1-2.7H1a2 2 0 0 1 0-4h.1A1.6 1.6 0 0 0 2.6 7a1.6 1.6 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1A1.6 1.6 0 0 0 7 2.6h.1A1.6 1.6 0 0 0 9 1.1V1a2 2 0 0 1 4 0v.1A1.6 1.6 0 0 0 15 2.6a1.6 1.6 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.6 1.6 0 0 0 1.1 2.7H23a2 2 0 0 1 0 4h-.1a1.6 1.6 0 0 0-1.5 1.1z" />
  </svg>
);

export const IconSearch = (): JSX.Element => (
  <svg {...base} width="16" height="16">
    <circle cx="11" cy="11" r="7" />
    <path d="M21 21l-4-4" />
  </svg>
);

export const IconPlay = (): JSX.Element => (
  <svg viewBox="0 0 24 24" fill="#04121a">
    <path d="M8 5v14l11-7z" />
  </svg>
);
