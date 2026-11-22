# TODO

Nur offene Arbeit. Erledigtes steht im [CHANGELOG.md](CHANGELOG.md), Hintergrund in
[ARCHITECTURE.md](ARCHITECTURE.md).

## Hoster

- **Streamtape / Luluvdo / LoadX.** Nur synthetisch getestet, kein Referenzcode. Live-Befund:
  aniworld und serienstream bieten diese Hoster aktuell nicht mehr an (nur VOE/Doodstream/
  Filemoon/Vidmoly/SpeedFiles), daher keine echten Embeds zum Nachziehen. Erst verifizierbar,
  wenn ein Zielportal sie wieder listet (Embed mit `dev/capture.sh` sichern).

## Netzwerk & Betrieb

- **Produktions-Rate-Limit.** Das eingebaute Limit ist Best-Effort im Prozessspeicher; für
  öffentlichen Betrieb gehört ein Edge-/Reverse-Proxy-Limit davor (verteilter Limiter mit
  Trusted-Proxy-`X-Forwarded-For`). Der In-Memory-Teil bleibt Defense-in-Depth.

## Download & Wiedergabe

- **Vollständig ffmpeg-freies HLS** umgesetzt: `VINDEX_NO_FFMPEG` finalisiert den nativen
  HLS-Download als rohen MPEG-TS (.ts) ohne ffmpeg. Offen (optional): echter TS→MKV-Remuxer für
  .mkv-Container ohne ffmpeg; SAMPLE-AES bleibt am ffmpeg-Pfad.
- **Syncplay** umgesetzt (Privacy-Flags + MPV_HOME-Härtung); nur der Lauf gegen echtes
  Syncplay ist noch Nutzer-seitig zu bestätigen.
- **Mehrstaffel-MAL-ID.** Die Zuordnung Staffel→MAL-ID höherer Staffeln ist eine ID-Sortier-
  Heuristik (`aniskip::resolve_mal_id`), gegen echte Mehrstaffler noch nicht verifiziert (Jikan
  war in der Testumgebung egress-geblockt).

## Desktop

- **Fenster-Frontend** (tauri oder egui) auf der `core`-Facade. Facade und Fortschritts-Kanal
  (`core::download::Progress`/`ProgressSink`, `Backend::download_with_progress`) sind da; offen
  ist nur das Fenster selbst (Event-Loop + Widgets), das wegen der System-Bibliotheken
  (WebKitGTK bzw. GPU) bewusst außerhalb des Workspaces liegt.

## Produkt-Entscheidungen (Default: so lassen)

- Suche ist reiner Substring (Punkt zählt); `--quality` deckelt Single-MP4-Hoster nicht nach
  oben; `naming::clean_title` schreibt klein.
