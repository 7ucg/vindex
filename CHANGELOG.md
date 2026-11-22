# Changelog

Alle nennenswerten Änderungen an diesem Projekt werden hier dokumentiert.

Das Format orientiert sich an [Keep a Changelog](https://keepachangelog.com/de/1.1.0/),
und dieses Projekt folgt [Semantic Versioning](https://semver.org/lang/de/).

## [Unreleased]

### Geplant

- TLS-Impersonate (`wreq`) und Headless-Browser-Fallback für Cloudflare-gated Seiten.
- Filemoon Byse-API (AES-256-GCM); Resolver für Streamtape, Luluvdo, LoadX.
- Batch-Download über Episodendatei, „keep watching", Proxy-Rotation.
- Watch via mpv/IINA, Syncplay, Anime4K.
- Nativer HLS-Downloader (ohne ffmpeg).
- API: Auth/API-Keys, Library, Auto-Sync, OpenAPI, Rate-Limit-Layer.
- Desktop-Fenster-Frontend auf der `core`-Facade.

## [0.1.0] - 2026-11-22

### Hinzugefügt

- Cargo-Workspace mit vier Crates: `vindex-core`, `vindex` (CLI), `vindex-api`,
  `vindex-desktop`.
- `core::model` — Typen für Sprache, Qualität (Tier-Logik), Hoster, Episode und
  Hoster-Links; Fehlertyp (thiserror).
- `core::http` — rotierender User-Agent-Pool und Cloudflare-/Captcha-Erkennung.
- `core::site` — Scraper für aniworld.to und s.to/serienstream.to (Staffeln, Episoden
  inkl. `[Episode N]`-Marker, Hoster-Links je Sprache), Scraper für filmo.to (Filme mit
  CSRF-/Redirect-Auflösung), sowie der `census`-Katalogzähler.
- `core::hoster` — Resolver für VOE, SpeedFiles, Vidoza, Doodstream, Vidmoly und
  Filemoon (Legacy-Pfad). Streamtape/Luluvdo/LoadX als offene Aufgaben.
- `core::extract` — HLS-Master-Parsing (m3u8-rs + Regex-Fallback), Qualitäts-Tiers.
- `core::download` — ffmpeg-Mux (Reconnect, Retry, Cleanup), Qualitäts-Sweep und
  Fallback-Kette Sprache → Hoster → Qualität, Abbruch über CancellationToken.
- `core::ratelimit` — Throttle pro Hoster (governor) und Backoff mit Jitter (backon).
- `core::config` — Einstellungen aus `VINDEX_*` und `.env`.
- `core::naming` — Datei-Templates (Jellyfin-Layout) und Episoden-Nummerierung.
- `core::aniskip` — Abruf der Intro/Outro-Zeiten.
- CLI mit den Unterbefehlen `census`, `search`, `download`, `menu` (interaktiv) und
  `serve`.
- HTTP-API-Daemon (axum): `/health`, `/api/search`, `/api/providers`, `/api/download`
  (Hintergrund-Queue), `/api/queue`.
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
- Noch offen: Revalidierung einzelner Redirect-Hops; der Daemon bindet per Default an
  localhost und hat noch keine Auth.

[Unreleased]: https://github.com/baron/vindex/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/baron/vindex/releases/tag/v0.1.0
