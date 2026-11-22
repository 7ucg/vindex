# Changelog

Alle nennenswerten Änderungen an diesem Projekt werden hier dokumentiert.

Das Format orientiert sich an [Keep a Changelog](https://keepachangelog.com/de/1.1.0/),
und dieses Projekt folgt [Semantic Versioning](https://semver.org/lang/de/).

## [Unreleased]

### Hinzugefügt

- CLI: Batch-Download über `vindex download --batch <datei>` — eine URL pro Zeile,
  Leerzeilen und mit `#` beginnende Kommentare werden übersprungen. Die URLs werden
  nacheinander mit demselben Sprach-/Hoster-/Qualitäts-Setup geladen (Delay dazwischen),
  ein Fehler bricht den Batch nicht ab; am Ende folgt eine Zusammenfassung mit den
  fehlgeschlagenen URLs.
- API: OpenAPI-3-Spezifikation (via `utoipa`), offen ausgeliefert unter
  `GET /openapi.json`. Annotiert `/health` und die vier `/api/*`-Routen inkl.
  Query-/Body-Parameter, Response-Codes (200/202/400/401/429/502) und
  API-Key-Sicherheitsschema. Das Laufzeitverhalten der Endpunkte bleibt gleich.

### Geplant

- TLS-Impersonate (`wreq`) und Headless-Browser-Fallback für Cloudflare-gated Seiten.
- Filemoon Byse-API (AES-256-GCM); Resolver für Streamtape, Luluvdo, LoadX.
- „keep watching", Proxy-Rotation.
- Watch via mpv/IINA, Syncplay, Anime4K.
- Nativer HLS-Downloader (ohne ffmpeg).
- API: Library, Auto-Sync, persistenter Key-Store mit Scopes.
- Desktop-Fenster-Frontend auf der `core`-Facade.

## [0.1.0] - 2026-11-22

### Hinzugefügt

- Cargo-Workspace mit vier Crates: `vindex-core`, `vindex` (CLI), `vindex-api`,
  `vindex-desktop`.
- `core::model` — Typen für Sprache, Qualität (Tier-Logik), Hoster, Episode und
  Hoster-Links; Fehlertyp (thiserror).
- `core::http` — rotierender User-Agent-Pool, Cloudflare-/Captcha-Erkennung und ein
  Retry-Helper, der bei transienten Fehlern (Verbindung/Timeout, Status 429/5xx) mit
  wachsendem Backoff wiederholt. Alle Portal- und Hoster-Fetches laufen darüber.
- `core::site` — Scraper für aniworld.to und s.to/serienstream.to (Staffeln, Episoden
  inkl. `[Episode N]`-Marker, Hoster-Links je Sprache), Scraper für filmo.to (Filme mit
  CSRF-/Redirect-Auflösung), sowie der `census`-Katalogzähler. URL-Autocorrect für von
  Hand getippte Links (Schema erzwingen, Host-Tippfehler korrigieren, Copy-Paste-Müll
  entfernen) und ein Spiegel-Fallback s.to ↔ serienstream.to bei Transport-Ausfall.
- `core::hoster` — Resolver für VOE, SpeedFiles, Vidoza, Doodstream, Vidmoly und
  Filemoon (Legacy-Pfad). Streamtape/Luluvdo/LoadX als offene Aufgaben.
- `core::extract` — HLS-Master-Parsing (m3u8-rs + Regex-Fallback), Qualitäts-Tiers.
- `core::download` — ffmpeg-Mux (Reconnect, Retry, Cleanup), Qualitäts-Sweep und
  Fallback-Kette Sprache → Hoster → Qualität, Abbruch über CancellationToken. Einzel-MP4-
  Streams werden per ffprobe auf ihre Höhe geprüft und unter der Mindestqualität
  übersprungen. Optional (`VINDEX_NATIVE_HLS`) holt vindex die HLS-Segmente selbst mit
  Retry/Backoff und lässt ffmpeg nur noch remuxen — robuster gegen 429-gedrosselte CDNs;
  verschlüsselte Playlists (`EXT-X-KEY`) fallen auf den ffmpeg-Pfad zurück.
- `core::ratelimit` — Throttle pro Hoster (governor) und Backoff mit Jitter (backon).
- `core::config` — Einstellungen aus `VINDEX_*` und `.env`.
- `core::naming` — Datei-Templates (Jellyfin-Layout) und Episoden-Nummerierung.
- `core::aniskip` — Abruf der Intro/Outro-Zeiten.
- CLI mit den Unterbefehlen `census`, `search`, `download`, `menu` (interaktiv) und
  `serve`.
- HTTP-API-Daemon (axum): `/health`, `/api/search`, `/api/providers`, `/api/download`
  (Hintergrund-Queue), `/api/queue`. Die `/api/*`-Routen sind pro Client-IP
  ratenbegrenzt (Sliding-Window, Standard 60 Req/min, `VINDEX_RATE_LIMIT`); über dem
  Limit `429` mit `Retry-After`. `/health` bleibt ausgenommen.
- Desktop-Backend-Facade auf `core`.
- Unit-Tests für die Hoster-Deobfuskation (VOE, SpeedFiles), HLS-Parsing,
  Qualitäts-Tiers, Naming/Episoden-Nummern, User-Agent-Rotation und URL-Validierung.

### Sicherheit

- Quell-URL-Validierung gegen eine Host-Allowlist (nur die unterstützten Portale, nur
  `https`) vor jedem serverseitigen Abruf — am API-Eingang (`400` bei Verstoß) und in
  der Download-Engine. Schützt den Daemon vor SSRF.
- Portal-Routing (`base_for_url`, `is_filmo`) entscheidet über den geparsten Host statt
  über Substring-Suche im Rohstring, damit ein fremder Portalname in Query/Pfad die
  Basis-URL nicht umlenken kann.
- Die Download-Engine übergibt ffmpeg nur `http`/`https`-Eingaben und setzt eine
  Protokoll-Allowlist, damit ein manipulierter Hoster ffmpeg nicht auf `file:` o. Ä.
  lenkt. Die HLS-Playlist wird über den konfigurierten Client (mit Timeout) und mit
  Größenlimit geladen.
- API-Key-Auth für den Daemon: `/api/*` verlangt bei gesetztem `VINDEX_API_KEYS` den
  Header `X-API-Key` bzw. `Authorization: Bearer` (konstanter Vergleich, `401` bei
  Fehler). Ohne Keys lässt sich nur auf localhost binden; ein öffentlicher Bind ohne
  Auth bricht den Start ab. Dazu ein Request-Rate-Limit pro Client-IP (Sliding-Window)
  gegen Flutung des Daemons, ein Queue-Limit, eine Obergrenze der gesamten Job-Liste und
  knappe Queue-Ausgaben ohne lokale Pfade oder interne Fehlertexte.
- Redirect-Policy für den HTTP-Client: jeder Hop muss `http`/`https` sein und darf nicht
  auf nicht-öffentliche IP-Ziele zeigen (loopback, privat, link-local, CGNAT, 0.0.0.0/8,
  192.0.0.0/24, 192.88.99.0/24, 198.18.0.0/15, unique-local sowie in IPv6 eingebettete
  IPv4 als mapped/6to4/NAT64), höchstens zehn Hops.
- DNS-Resolver mit Adress-Filter: jede per DNS aufgelöste Zieladresse wird gegen dieselbe
  Sperrliste geprüft, bevor verbunden wird. Schließt DNS-Rebinding (ein Hostname, der auf
  eine interne IP zeigt) zusätzlich zur Redirect-Prüfung der IP-Literale.
- Noch offen: persistenter Key-Store mit Scopes, Login-Throttle und SSO (bisher nur
  statische Keys aus der Umgebung).

[Unreleased]: https://github.com/baron/vindex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/baron/vindex/releases/tag/v0.1.0
