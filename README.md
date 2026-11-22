# vindex

Downloader und Streamer für deutsche Streaming-Portale (aniworld.to,
s.to / serienstream.to, filmo.to), in Rust. Eine Kern-Library, darauf CLI,
HTTP-API/Daemon und Desktop-Backend.

> Status: frühe Alpha. Scraping, Hoster-Auflösung, Stream-Extraktion,
> Download-Engine, CLI und HTTP-API-Daemon sind da und bauen grün. Die
> Download-Pfade hängen an Live-Seiten, Hostern und Cloudflare und sind deshalb
> wartungsintensiv. Mehr in [ARCHITECTURE.md](ARCHITECTURE.md) und
> [CHANGELOG.md](CHANGELOG.md).

## Funktionen

- Portale: aniworld.to (Serien, Staffeln, Episoden samt `Filme`-Tab), serienstream.to
  (Serien unter `/serie/<slug>`) und filmo.to (Filme unter `/movies/<slug>`) funktionieren —
  jeweils live end-to-end bis zur spielbaren Stream-URL verifiziert. s.to ist stillgelegt.
- Katalogsuche, Parsing von Slug, Staffel und Episode, Hoster-Auflösung über
  Redirects.
- Stream-Extraktion je Hoster: VOE (inkl. voe.sx-Stub-Redirect zur rotierenden CDN-Domain),
  Vidmoly, Vidoza, Doodstream, SpeedFiles, Filemoon (moderne Byse-API mit AES-256-GCM,
  Legacy-packed-JS als Fallback).
- Sprachauswahl (German Dub, German Sub, English Sub, je nach Portal) mit
  Fallback über Hoster und Sprache.
- Qualitäts-Sweep zwischen Max- und Min-Grenze (4K bis 360p).
- Download-Engine auf ffmpeg-Basis, mit Retry, Reconnect, Abbruch und
  Fortschrittsmeldungen (CLI-Statuszeile, Progress-Kanal für das Desktop-Backend);
  optionaler nativer HLS-Segment-Download (`VINDEX_NATIVE_HLS`) inkl. AES-128-Entschlüsselung.
- Naming-Templates im Jellyfin-Layout, Rate-Limiting pro Hoster, Backoff,
  User-Agent-Rotation, Cloudflare-Erkennung.
- TLS-Impersonate-Fallback (Anti-Bot Stufe 2) über `wreq` mit Chrome-Fingerprint —
  opt-in über `VINDEX_IMPERSONATE`, standardmäßig deaktiviert.
- Headless-Browser-Fallback (Anti-Bot Stufe 3) über `obscura` — opt-in über `VINDEX_OBSCURA`
  (Pfad zum Binary). obscura läuft hinter einem erzwungenen 1.1.1.1-Resolver-Proxy.
- Konfiguration über `VINDEX_*`-Variablen und `.env`.
- AniSkip-Abruf für Intro- und Outro-Zeiten.
- Watch-Modus: `vindex watch <url>` spielt eine Episode/einen Film direkt in mpv ab
  (statt Download), mit Referer-Header und AniSkip als überspringbare Kapitel.
- Schnittstellen: CLI mit interaktivem Menü, HTTP-API-Daemon, Desktop-Backend.

Was noch fehlt (Headless-Browser-Fallback, einzelne Hoster,
Syncplay, weitere API-Flächen, Desktop-Frontend), steht in
[ARCHITECTURE.md](ARCHITECTURE.md) und [CHANGELOG.md](CHANGELOG.md).

## Nutzung

### CLI

```bash
# Katalog zählen
vindex census --catalog-only
vindex census --episodes --output census.csv

# Suchen
vindex search "naruto"

# Episode/Film laden (Sprache/Hoster/Qualität mit Fallback)
vindex download "https://aniworld.to/anime/stream/<slug>/staffel-1/episode-1" \
  --language "German Dub" --provider VOE --quality 1080p
vindex download "https://filmo.to/movies/<slug>"

# Mehrere Episoden aus einer Datei (eine URL pro Zeile, # = Kommentar)
vindex download --batch episoden.txt --language "German Dub" --quality 1080p

# Nächste Folge einer Serie (merkt sich den Fortschritt pro Serie)
vindex download --continue "https://aniworld.to/anime/stream/<slug>"

# Episode/Film direkt in mpv abspielen statt laden (Referer + AniSkip-Kapitel)
vindex watch "https://aniworld.to/anime/stream/<slug>/staffel-1/episode-1" \
  --language "German Dub" --provider VOE
# Player wählbar: --player mpv (Standard) oder --player iina (macOS).
# Optional Anime4K-Upscaling: --anime4k A --anime4k-dir /pfad/zu/shadern
# Voraussetzung: mpv (bzw. iina-cli) ist installiert und im PATH.

# Fertige Downloads im Zielverzeichnis auflisten
vindex library

# Interaktives Menü
vindex menu

# HTTP-API-Daemon starten
vindex serve --addr 127.0.0.1:8080
```

### Als Library

```toml
[dependencies]
vindex-core = "0.1"
```

```rust
use vindex_core::{config::Settings, download::{download_url, DownloadPrefs}, http};
// Episode/Film auflösen und herunterladen …
```

### HTTP-API

```bash
vindex serve --addr 127.0.0.1:8080
curl "http://127.0.0.1:8080/api/search?keyword=naruto"
curl -X POST -H "Content-Type: application/json" \
  -d '{"url":"https://aniworld.to/anime/stream/x/staffel-1/episode-1","language":"German Dub","provider":"VOE"}' \
  http://127.0.0.1:8080/api/download
curl http://127.0.0.1:8080/api/queue
curl http://127.0.0.1:8080/api/library   # fertige Downloads im Zielverzeichnis
```

Die OpenAPI-3-Spezifikation (via `utoipa`) liegt offen — wie `/health`, ohne
API-Key — unter `/openapi.json` und beschreibt alle Routen, Parameter und
Response-Codes:

```bash
curl http://127.0.0.1:8080/openapi.json
```

Jede Quell-URL wird gegen eine Host-Allowlist geprüft (nur die unterstützten
Portale); alles andere lehnt die API mit `400` ab.

Sind über `VINDEX_API_KEYS` (komma-getrennt) Keys gesetzt, verlangen alle
`/api/*`-Routen den Header `X-API-Key: <key>` (oder `Authorization: Bearer <key>`);
ohne gültigen Key antwortet die API mit `401`. Ohne gesetzte Keys startet der Daemon
nur auf `127.0.0.1` — ein öffentlicher Bind ohne Auth wird abgelehnt.

Für feinere Rechte gibt es einen persistenten Key-Store (`VINDEX_API_KEYS_FILE`, JSON):
pro Key ein `read`/`write`/`admin`-Scope, gespeichert nur als sha256-Hash. GET-Routen
brauchen `read`, `POST /api/download` `write`; zu wenig Rechte ergeben `403`. Die
Env-Keys gelten als `admin`-Bootstrap. Keys lassen sich mit einem Admin-Key über
`/api/keys` verwalten: `POST` legt einen an (Klartext einmalig in der Antwort), `GET`
listet `{name, scope}`, `DELETE /api/keys/<name>` widerruft.

```bash
VINDEX_API_KEYS=geheim123 vindex serve --addr 0.0.0.0:8080
curl -H "X-API-Key: geheim123" "http://127.0.0.1:8080/api/search?keyword=naruto"
```

Die `/api/*`-Routen sind zusätzlich pro Client-IP ratenbegrenzt (Sliding-Window,
Standard 60 Requests/Minute, über `VINDEX_RATE_LIMIT` einstellbar). Über dem Limit
antwortet die API mit `429` und dem Header `Retry-After`. `/health` ist ausgenommen.

## Build

```bash
cargo build --release --workspace
cargo test --workspace
```

## Workspace-Struktur

```
vindex/
├─ crates/
│  ├─ core/      # Library: Scraper, Hoster-Resolver, Modell, Download-Engine
│  ├─ cli/       # CLI + interaktives Menü (Binary: vindex)
│  ├─ api/       # HTTP-API / Daemon (Binary: vindex-api)
│  └─ desktop/   # Backend-Facade auf core
└─ ARCHITECTURE.md
```

## Voraussetzungen

| Tool | Zweck | Pflicht? |
| --- | --- | --- |
| Rust (stable, Edition 2021) | Build | ja |
| ffmpeg / ffprobe | Muxing HLS/mp4, Auflösungs-Erkennung | für Downloads |
| mpv (bzw. IINA auf macOS) | Streaming / Watch | optional |
| yt-dlp | generischer Download-Fallback | optional |

## Credits

Aufbau und Portal-Logik lehnen sich an bestehende Open-Source-Downloader an:

- AniWorld-Downloader (`phoenixthrush/AniWorld-Downloader`): Modelle, Extraktoren,
  AniSkip, HTTP-API-Design.
- Aniloader (`NINJAMINEBRO`): Hoster-Deobfuskation, Cloudflare-Fallback,
  Quality-Sweep.

## Lizenz

MIT, siehe [LICENSE](LICENSE).


