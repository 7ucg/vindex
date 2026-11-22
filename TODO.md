# TODO

Offene Arbeit an vindex, grob nach Priorität. Hintergrund und Entwurfsgründe stehen in
[ARCHITECTURE.md](ARCHITECTURE.md); erledigte Punkte stehen im [CHANGELOG.md](CHANGELOG.md).

## Live-Verifikation (braucht echte Portale / Cloudflare)

Diese Teile sind implementiert und (gegen Fixtures/netzfrei) getestet, aber NICHT gegen
die echten Seiten bestätigt — das geht nur in einer Umgebung mit Zugriff und ggf. mit
`VINDEX_IMPERSONATE=1`:

- **serienstream.to-Parser.** Das neue Markup (`data-provider-name`/`data-play-url`
  `/r?t=<token>`) wird geparst. Offen: ob der Abruf durch Cloudflare kommt und der
  `/r?t=`-Redirect real zum Hoster führt.
- **Resolver Streamtape / Luluvdo / LoadX.** Nach dokumentiertem Baumuster gebaut, nur
  synthetisch getestet. Exakte Keys/Regexe gegen echte Embeds nachziehen (Embeds mit
  `dev/capture.sh` sichern, sobald ein Portal sie anbietet). LoadX ist am unsichersten.
- **TLS-Impersonate (Stufe 2).** `VINDEX_IMPERSONATE=1` gegen die echten CF-Seiten testen;
  prüfen, ob der Chrome149-Fingerprint durchkommt.

## Netzwerk & Anti-Bot

- **Headless-Browser-Fallback (Stufe 3)** erledigt: obscura (`core::browser`, opt-in
  `VINDEX_OBSCURA`) hinter einem erzwungenen 1.1.1.1-CONNECT-Proxy mit SSRF-Filter. Live gegen
  serienstream.to durch Cloudflare verifiziert. Offen: der Proxy braucht einen Listening-Socket
  (in manchen Sandboxes blockiert) und obscura muss installiert sein.

## Hoster-Pflege

- **Filemoon Byse-API** erledigt **und live verifiziert**: `/api/videos/<code>` wird zuerst
  versucht, die AES-256-GCM-`playback`-Daten werden entschlüsselt (Key aus `key_parts` + Version),
  Legacy-packed-JS bleibt Fallback. Gegen eine echte Byse-Antwort bestätigt (30 `key_parts`,
  `version:"7"` als String, 12-Byte-IV) — der Decrypt liefert die reale HLS-master-URL. IV-Längen
  ≠ 12 Byte (nicht-96-Bit-GCM) sind bewusst nicht abgedeckt.

## API

- **Weitere Flächen.** `GET /api/library` (listet die fertigen Downloads im Zielverzeichnis,
  nur Dateiname + Größe, read-Scope) ist erledigt. Offen: Auto-Sync (Abgleich des Bibliotheks-
  Stands über mehrere Clients). (OpenAPI unter `/openapi.json`, das Request-Rate-Limit, der
  persistente Key-Store mit Scopes und die admin-Key-Verwaltung unter `/api/keys` sind erledigt.)
- **Produktions-Rate-Limit.** Das eingebaute Limit ist Best-Effort im Prozessspeicher; für
  öffentlichen Betrieb gehört ein Edge-/Reverse-Proxy-Limit davor (verteilter Limiter mit
  Trusted-Proxy-`X-Forwarded-For`). Der In-Memory-Teil bleibt Defense-in-Depth.

## Download & Wiedergabe

- **Vollständig ffmpeg-freies HLS.** Der Segment-Download (`VINDEX_NATIVE_HLS`) und die
  AES-128-Entschlüsselung laufen nativ, nur das Remuxen nutzt noch ffmpeg. Ein eigener
  TS-Remuxer würde ffmpeg ganz ersetzen; `SAMPLE-AES` bleibt vorerst beim ffmpeg-Pfad.
- **Watch** via mpv erledigt (`vindex watch`): Stream-Auflösung ohne Download
  (`core::download::resolve_playable`), mpv-Start mit Referer-Header und Medientitel, AniSkip
  als überspringbare Kapitel (`core::mpv::build_mpv_args` → `--chapters-file=memory://…`).
  Die MAL-ID-Auflösung läuft jetzt über Jikan (`aniskip::resolve_mal_id`): Slug/Staffel aus der
  URL, Titelsuche, Relations-Kette für höhere Staffeln; AniSkip liefert damit echte Intro/Outro-
  Kapitel. Player-Auswahl `--player mpv|iina` und Anime4K-Upscaling `--anime4k <modus>`
  (`core::mpv::Player`/`build_player_args`/`Anime4kMode`/`anime4k_glsl_value`) sind erledigt.
  **Syncplay zurückgestellt:** es teilt die tokenisierte Stream-URL mit einem Dritt-Server/Raum
  und reicht die mpv-Härtungsflags nur unverifiziert weiter — kommt erst zurück, wenn das gegen
  echtes Syncplay geprüft und fail-closed abgesichert ist (Privacy-Flags + garantierte Härtung
  via `MPV_HOME`-Config). Außerdem offen: die Staffel→MAL-ID-Zuordnung höherer Staffeln ist eine
  ID-Sortier-Heuristik und gegen echte Mehrstaffler noch nicht verifiziert.

## Desktop

- **Fenster-Frontend** (tauri oder egui) auf der `core`-Facade. Die Facade reicht Suche,
  Provider, Download, Config und Hoster-Auflösung durch; das **Fortschritts-Reporting** ist
  erledigt (`core::download::Progress`/`ProgressSink`, durch die ganze Download-Kette verdrahtet,
  `Backend::download_with_progress`). Offen bleibt nur das Fenster selbst (Event-Loop + Widgets),
  das wegen der System-Bibliotheken (WebKitGTK bzw. GPU) bewusst außerhalb des Workspaces liegt.

## Tests & Produkt-Entscheidungen

- **Scraper-Fixtures** erledigt für die implementierten Teile: Episoden-Parser
  (`parse_episodes`, inkl. Filme-Tab/Staffel 0), Staffel-Navigation (`parse_season_nav`), beide
  Hoster-Markups und alle aktiven Extraktoren (VOE, SpeedFiles, Vidoza, Vidmoly, Doodstream,
  Filemoon-Byse) sind mit Roundtrip-, Fallback- und Fehlerpfad-Tests gegen synthetische Fixtures
  abgesichert; Queue-Cap über `try_admit_job` mit `429`-Unit-Tests. Offen bleibt nur, was live-
  Embeds braucht: Streamtape/Luluvdo/LoadX (kein Referenzmaterial, siehe Live-Verifikation).
- **Offene Entscheidungen** (Default: so lassen): Suche ist reiner Substring (Punkt zählt);
  `--quality` deckelt Single-MP4-Hoster nicht nach oben; `naming::clean_title` schreibt klein.

## Entwicklungs-Werkzeuge

- `dev/capture.sh <url> <datei>` sichert Live-Seiten über DoH (1.1.1.1) nach `dev-fixtures/`
  (gitignored) — Grundlage, um neue Hoster-/Portal-Varianten offline zu analysieren.
