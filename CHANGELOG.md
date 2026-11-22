# Changelog

Alle nennenswerten Änderungen an diesem Projekt werden hier dokumentiert.

Das Format orientiert sich an [Keep a Changelog](https://keepachangelog.com/de/1.1.0/),
und dieses Projekt folgt [Semantic Versioning](https://semver.org/lang/de/).

## [Unreleased]

### Hinzugefügt

- Watch: Anime4K-Upscaling `--anime4k <modus>` (`core::mpv::Anime4kMode`/`anime4k_glsl_value`).
  Die offiziellen GLSL-Shader-Ketten (A, B, C, A+A, B+B, C+A) werden als `--glsl-shaders` an
  mpv (bzw. `--mpv-glsl-shaders` an IINA) übergeben; die Shader liegen im Verzeichnis
  `--anime4k-dir` bzw. `VINDEX_ANIME4K_DIR`.
- Watch: Player-Auswahl `--player mpv|iina` (`core::mpv::Player`). IINA läuft über `iina-cli`
  und bekommt dieselben (gehärteten) mpv-Optionen als `--mpv-…`; die URL bleibt ein eigenes
  letztes Argument. Standard bleibt mpv.
- Download: Fortschritts-Reporting (`core::download::Progress`/`ProgressSink`). Ein optionaler,
  threadsicherer Sink wird durch die ganze Kette (`download_url`/`_episode`/`_movie` →
  `try_download_resolved` → nativer HLS-Segment-Loop) gereicht und meldet Auflösen, Start mit
  Segmentzahl, jedes fertige Segment (inkl. Bytes), Remux und Abschluss. Die CLI zeigt daraus
  eine einzeilige Statuszeile; die Desktop-Facade bekommt `download_with_progress` fürs GUI. Ohne
  Sink bleibt die Engine unverändert.
- Hoster/Filemoon: moderner Byse-API-Pfad (`/api/videos/<code>`). Die AES-256-GCM-
  verschlüsselten `playback`-Daten werden entschlüsselt — der 32-Byte-Key wird aus `key_parts`
  und der `version` zusammengesetzt (versionsabhängige Hälften, beide Reihenfolgen als
  Kandidaten), beide Feldpaare (iv/payload, iv2/payload2) werden probiert. Aus dem JSON wird
  die höchstaufgelöste Quelle gewählt. Schlägt die API fehl, greift weiter der Legacy-packed-JS-
  Pfad. `decrypt_playback`, `playback_keys` und `best_source_url` sind reine, per AES-GCM-Round-
  Trip getestete Funktionen; externe Antworten werden größenbegrenzt gelesen.
- Watch/AniSkip: MAL-ID-Auflösung über Jikan (`aniskip::resolve_mal_id`). `vindex watch`
  leitet Slug/Staffel/Episode aus der URL ab, sucht den MyAnimeList-Eintrag über die Jikan-
  Titelsuche und zieht für höhere Staffeln die Relations-Kette (Sequel/Prequel/Side story)
  nach. Damit liefert AniSkip echte Intro/Outro-Zeiten, die als überspringbare mpv-Kapitel
  eingehängt werden. Parsing und Auswahl (`parse_search`, `pick_season1`, `parse_related_ids`,
  `slug_to_query`) sind reine, netzfrei getestete Funktionen; jeder Fehlschlag (keine Episode,
  kein Treffer, API nicht erreichbar) spielt still ohne Kapitel weiter statt abzubrechen.
- API: admin-geschützte Key-Verwaltung unter `/api/keys`. `POST` legt einen Key an (gibt den
  Klartext einmalig zurück, speichert nur den sha256-Hash), `GET` listet `{name,scope}` (nie
  Hashes/Klartext), `DELETE /api/keys/{name}` widerruft. Änderungen wirken sofort (In-Memory-
  Store wird neu geladen), die Store-Datei wird atomar mit 0600 geschrieben. Wird zur Laufzeit
  der letzte Key gelöscht, sperrt die API (deny-all) statt zu öffnen.
- CLI: Watch-Modus — `vindex watch <episode-url>` löst den Stream über dieselbe
  Sprach-/Hoster-Fallback-Kette wie `download` auf (neue `core::download::resolve_playable`,
  ohne Mux) und spielt ihn direkt in mpv ab, statt ihn herunterzuladen. Der Referer wird als
  `--http-header-fields=Referer: …` mitgegeben, der Medientitel als `--force-media-title`.
  Der Start läuft über `tokio::process::Command` mit getrennten Argumenten (keine Shell); die
  Stream-URL ist ein eigenes Argument, fehlendes mpv ergibt einen sauberen Fehler statt Panic.
  AniSkip-Zeiten werden über `core::mpv::AniSkipTimes` als überspringbare Kapitel eingehängt
  (`--chapters-file=memory://…` im ffmetadata-Format).
- Proxy-Rotation: `VINDEX_PROXY` (komma-getrennt) verteilt die Anfragen round-robin über
  einen Proxy-Pool (reqwest- und Impersonate-Client). Fail-closed — ein kaputter Proxy
  bricht den Client-Bau ab, statt ungeproxt weiterzulaufen. Hinweis: über einen Proxy
  greift der eigene DNS-/IP-Filter nicht mehr (der Proxy löst auf); das ist eine bewusste
  Operator-Entscheidung, die Host-Allowlist am Eingang bleibt.
- CLI: „keep watching" — nach jedem Download wird pro Serie die zuletzt geladene Folge
  gemerkt (`VINDEX_WATCH_STATE`-JSON), und `vindex download --continue <serien-url>` lädt
  die nächste Folge (oder Staffel 1 / Episode 1, wenn nichts gemerkt).
- API: persistenter Key-Store mit Scopes. Keys liegen als sha256-Hash (nie im Klartext) in
  einer JSON-Datei (`VINDEX_API_KEYS_FILE`); jeder Key hat einen Scope `read`/`write`/`admin`
  (GET braucht `read`, `POST /api/download` `write`, Admin deckt alles). Zu niedriger Scope
  → `403`. `VINDEX_API_KEYS` (Env) bleibt als Admin-Bootstrap. Ein Store-Ladefehler sperrt
  die API (deny-all) statt sie zu öffnen; `generate_key` nutzt einen CSPRNG.
- Nativer HLS-Pfad (`VINDEX_NATIVE_HLS`) entschlüsselt jetzt AES-128-CBC-Playlists selbst
  (Schlüssel über denselben Client + SSRF-Prüfung, IV deklariert oder aus der
  Mediensequenz, PKCS7). `SAMPLE-AES` und andere Methoden fallen weiter auf ffmpeg zurück.
- Anti-Bot Stufe 2: TLS-Impersonate-Fallback über `wreq` (BoringSSL-JA3/JA4-
  Fingerprint). Opt-in über `VINDEX_IMPERSONATE` (Default **aus**). Sieht eine
  Portal-Antwort nach einer Cloudflare-Challenge aus (Status 403/429/503 oder
  Interstitial-Marker), wird die Seite einmalig über einen Chrome-Fingerprint-Client
  nachgeholt. Der Impersonate-Pfad nutzt denselben festen DNS-Upstream (1.1.1.1) und
  Adressfilter wie der reqwest-Pfad (SSRF-/Rebinding-Schutz). Ohne das Flag unverändert.
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

- Headless-Browser-Fallback (Stufe 3) für Cloudflare-gated Seiten, die auch der
  TLS-Impersonate nicht durchbricht.
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
