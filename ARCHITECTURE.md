# Architektur — vindex

Technische Referenz: Aufbau und Entwurfsentscheidungen von vindex, einem
Downloader/Streamer für deutsche Streaming-Portale (aniworld.to, s.to /
serienstream.to, filmo.to) in Rust. Dieselbe Kernlogik läuft als Library, CLI,
HTTP-API/Daemon und Desktop-Backend.

Crate-Versionen wurden auf crates.io geprüft (Stand 2026-11-22); vor einem Release
erneut verifizieren.

---

## 1. Zweck & Scope

**Ziel-Nutzungsformen**

| Form | Crate | Beschreibung |
| --- | --- | --- |
| Library | `vindex-core` | Scraper, Hoster-Resolver, Modell, Download-Engine (publishbar). |
| CLI | `vindex` | `clap`-Befehle + interaktives Menü. |
| API/Daemon | `vindex-api` | `axum`-REST-API mit Download-Queue und Status. |
| Desktop | `vindex-desktop` | Backend-Facade auf `core` (GUI-Frontend folgt). |

**Portale & Hoster-Stand**

| Hoster | Resolver | Quelle |
| --- | --- | --- |
| VOE | vorhanden (HLS + mp4, Deobfuskation) | models/extractors |
| Vidmoly | vorhanden (HLS) | models/extractors |
| Vidoza | vorhanden (mp4) | models/extractors |
| Doodstream | vorhanden (mp4, pass_md5 + token) | models/extractors |
| Filemoon | vorhanden (Legacy packed-JS; Byse-API offen) | models/extractors |
| SpeedFiles | vorhanden (10-Schritt-Deobfuskation) | Aniloader |
| Streamtape / Luluvdo / LoadX | kein Resolver | keine Referenz-Vorlage |

Streamtape, Luluvdo und LoadX sind im Scope, haben aber keine Vorlage und sind als
eigene Reverse-Engineering-Aufgaben offen (siehe §8).

**Sprach-Varianten** (aniworld `data-lang-key`): `1` German Dub, `2` Japanese · English
Sub, `3` Japanese · German Sub. s.to bietet zusätzlich English Dub. Filmo: German Dub,
English Dub.

---

## 2. Crate-Entscheidungen

### 2.1 HTTP / Anti-Bot / TLS-Fingerprint

| Crate | Version | Zweck | Entscheidung |
| --- | --- | --- | --- |
| `reqwest` | 0.13 | Standard-Async-HTTP | Basis-Transport. |
| `wreq` + `wreq-util` | 6.0.0-rc / 3.0.0-rc | Browser-Impersonate (TLS/JA3/JA4, HTTP2) | Stufe 2 umgesetzt, opt-in (`VINDEX_IMPERSONATE`, Default aus). BoringSSL via `btls-sys` (braucht cmake/clang zum Bauen). |
| `scraper` | 0.29 | HTML/CSS-Selektoren | Scraping. |
| `regex` | 1 | Deobfuskation, Playlist-Parsing | — |

`rquest` ist nach `wreq` umbenannt. Cloudflare-Challenge-Solver-Crates (ghostwire,
cloudscraper-rs, chaser-cf, impersonate-rs) sind unreif und werden nicht als
Produktiv-Abhängigkeit geführt; die Strategie ist gestuft (§5).

### 2.2 Scraping / Medien

| Crate | Version | Zweck |
| --- | --- | --- |
| `m3u8-rs` | 6 | HLS-Master-Playlist-Parsing (mit Regex-Fallback). |
| `base64` | 0.22 | VOE-/SpeedFiles-Decode. |
| `serde` / `serde_json` | 1 | Modelle, Config, API, AniSkip. |

### 2.3 Rate-Limiting / Retry

| Crate | Version | Zweck |
| --- | --- | --- |
| `governor` | 0.10 | GCRA-Client-Throttle pro Hoster. |
| `backon` | 1 | exponentieller Backoff mit Jitter (`backoff` ist unmaintained). |
| `tower` | 0.5 | Middleware der API-Schicht. |

### 2.4 Async / CLI / API / Config

| Crate | Version | Zweck |
| --- | --- | --- |
| `tokio` / `tokio-util` | 1 / 0.7 | Runtime, `CancellationToken`. |
| `futures` | 0.3 | Stream-Kombinatoren. |
| `clap` / `indicatif` / `dialoguer` | 4 / 0.18 / 0.12 | CLI, Fortschritt, interaktives Menü. |
| `axum` | 0.8 | HTTP-API. |
| `config` / `dotenvy` / `tracing` | 0.15 / 0.15 / 0.1 | Einstellungen, `.env`, Logging. |
| `anyhow` / `thiserror` | 1 / 2 | Fehler in Binaries bzw. Library. |

---

## 3. Workspace- & Modul-Architektur

```
vindex/
├─ Cargo.toml              # [workspace]
├─ crates/
│  ├─ core/                # Library
│  │  └─ src/
│  │     ├─ model.rs       # Language, Quality, Hoster, Episode, ProviderLinks
│  │     ├─ error.rs       # Error/Result (thiserror)
│  │     ├─ http.rs        # Client, UA-Rotation, Cloudflare-Erkennung
│  │     ├─ site/          # aniworld, serienstream (generisch), sto, filmo + Census
│  │     ├─ hoster/        # voe, speedfiles, vidoza, doodstream, vidmoly, filemoon
│  │     ├─ extract.rs     # HLS-Master-Parsing, Quality-Tiers
│  │     ├─ download.rs    # ffmpeg-Mux, Qualitäts-Sweep, Fallback-Kette
│  │     ├─ ratelimit.rs   # governor pro Hoster, backon-Backoff
│  │     ├─ naming.rs      # Datei-Templates, Episoden-Nummerierung
│  │     ├─ config.rs      # VINDEX_*-Einstellungen
│  │     └─ aniskip.rs     # Intro/Outro-Zeiten
│  ├─ cli/                 # clap + dialoguer; Unterbefehle census/search/download/menu/serve
│  ├─ api/                 # axum-Daemon
│  └─ desktop/             # Backend-Facade auf core
└─ ARCHITECTURE.md
```

---

## 4. Mapping der Portal-Logik

### 4.1 SerienStream (aniworld.to / s.to)

| Aufgabe | Selektor / Logik |
| --- | --- |
| Katalog | `GET /animes-alphabet`, `a[href^="/anime/stream/"]` (Basis-Slug). |
| Staffeln | `div.hosterSiteDirectNav ul li a:not([data-episode-id])`; `Filme` → Sentinel-Staffel 0. |
| Episoden | `tbody#season{N} tr td.seasonEpisodeTitle a span` → Titel + href; `[Episode NNN]`-Marker abschneiden. |
| Hoster-Links | `li[data-lang-key][data-link-target]` mit `<h4>` = Hoster-Name → `…/redirect/N`. |

`scraper::Html` ist nicht `Send`: Daten extrahieren und das Dokument vor dem nächsten
`.await` fallenlassen.

### 4.2 Filmo (filmo.to)

Eigener Pfad: Die Filmseite listet pro Sprache Hoster-Chips (`[data-provider-chip]`
mit `data-p`-Token). Auflösung: CSRF-geschützter `POST /n` mit `{"p": token}` →
Playback-Token `x` → `GET /n/{token}` folgt dem Redirect zur Hoster-Embed-URL, die der
normale Hoster-Resolver übernimmt.

### 4.3 Hoster-Deobfuskation

- **VOE**: `<script type="application/json">` → ROT13 → Junk-Sequenzen entfernen →
  base64 → Shift(−3) → reverse → base64 → JSON `source`; Fallbacks `a168c`, `'hls'`, bare `.m3u8`.
- **SpeedFiles**: `var _0x5opu234` → 10-Schritt-Kette (base64/swapcase/reverse/hex-Paare/Shift).
- **Vidoza**: `src:"…"`. **Vidmoly**: `file:"…m3u8…"` (Referer vidmoly.biz).
- **Doodstream**: `pass_md5`-URL + token → Basis-URL + random + `?token=…&expiry=…`.
- **Filemoon**: packed-JS entpacken → `sources/file/.m3u8`.

### 4.4 Qualität & Mux

Tiers hoch→niedrig: 4K ≥1440, 1080p ≥900, 720p ≥600, 480p ≥420, 360p ≥0. Sweep von
Max- bis Min-Tier; HLS-Master → exakte Tier-URL; Single-MP4 → best effort (ffprobe-Gate
optional). Mux via `ffmpeg -reconnect … -c copy`; Retries mit wachsendem Delay, Abbruch
über `CancellationToken`.

---

## 5. Anti-Bot- / Rate-Limit-Strategie

Gestuft, Höflichkeit vor Umgehung:

1. **Fast-Path**: `reqwest` mit browser-ähnlichen Headern, rotierender User-Agent.
2. **TLS-Impersonate** (umgesetzt, opt-in via `VINDEX_IMPERSONATE`, Default aus): bei
   Cloudflare-Verdacht einmalige Wiederholung über `wreq` mit Chrome-Fingerprint
   (`wreq-util::Emulation::Chrome149`). Hook in der Fetch-Schicht
   (`site::fetch_text` und `serienstream::provider_links`). Der wreq-Client nutzt
   über `wreq::dns::Resolve` denselben 1.1.1.1-Upstream + Adressfilter wie der
   reqwest-Pfad, dazu Schema-/IP-Literal-Vorprüfung — SSRF-/Rebinding-Schutz bleibt.
3. **Headless-Browser-Fallback**: echter Browser als letzter Ausweg (offen).

Challenge-Erkennung: Status `403/429/503` oder Body-Marker (`just a moment`,
`verifying you are human`, `cf-browser-verification`, `challenge-platform`, `cf_chl`,
`captcha`, …). Rate-Limit: `governor` pro Hoster-Domain, `backon`-Backoff mit Jitter,
konfigurierbares Download-Delay zwischen Queue-Starts.

---

## 6. Externe Tools

- **ffmpeg/ffprobe**: Standard-Mux-Engine (HLS/mp4, Untertitel, Reconnect).
- **mpv/IINA**: für späteres Streaming/Watch + AniSkip.
- Optional: **yt-dlp** als generischer Fallback; ein eigener HLS-Downloader (ffmpeg-frei)
  als späteres Feature.

---

## 7. Konfiguration

`VINDEX_*`-Umgebungsvariablen (plus `.env`). Abgedeckt u. a.: Download-Pfad, Sprache,
Provider + Fallback-Reihenfolge, Qualitäts-Grenzen, gleichzeitige Downloads,
Download-Delay, Retries, AniSkip, Naming-Template (Jellyfin-Layout).

---

## 8. Risiken

Cloudflare ändert seine Challenges laufend. Der Browser-Fallback ist dagegen der
Sicherheitsanker; die TLS-Fingerprints veralten und müssen nachgezogen werden.

Das HTML der Portale ist instabil. Selektoren wie `data-lang-key`,
`seasonEpisodeTitle` oder die VOE-Payload brechen bei einem Redesign, deshalb
liegen sie zentral, parsen defensiv und laufen gegen echte Seiten im Test.

Jeder Hoster kann seinen Auslieferungsweg wechseln, also braucht jeder Resolver
Versionierung und Fallback-Patterns. Die Filemoon Byse-API (AES-256-GCM) steht
dafür noch aus, und für Streamtape, Luluvdo und LoadX fehlt bislang eine Vorlage.

Rechtlich: nur legale und private Nutzung. TLS-Impersonate, Browser-Fallback und
etwaige Captcha-Funktionen sind AGB-kritisch und standardmäßig deaktiviert.

---

## 9. Status & Roadmap

Umgesetzt: Workspace, Modell, HTTP + Cloudflare-Erkennung, SerienStream- und
Filmo-Scraper, sechs Hoster-Resolver, HLS-Parsing + Qualitäts-Sweep, ffmpeg-Download
mit Fallback-Kette, Rate-Limiting, Config, AniSkip-Abruf, CLI (census/search/download/
menu/serve) und ein HTTP-API-Daemon (Suche/Provider/Download/Queue) mit
API-Key-Auth, Queue-Limit und localhost-Bind-Schutz.

Offen (u. a.):

1. Headless-Browser-Fallback (Stufe 3). TLS-Impersonate (Stufe 2, `wreq`) ist
   umgesetzt (opt-in).
2. Filemoon Byse-API (AES-256-GCM); Streamtape/Luluvdo/LoadX.
3. Batch-Datei, „keep watching", Proxy-Rotation.
4. Watch via mpv/IINA, Syncplay, Anime4K.
5. Nativer HLS-Downloader (ffmpeg-frei).
… (10.) weitere API-Flächen: Library, Auto-Sync, OpenAPI, Rate-Limit-Layer.
(11.) Erweiterte API-Auth: persistenter Key-Store mit Scopes, Login-Throttle, SSO
(statische Keys aus der Umgebung sind vorhanden); weitere Endpunkte.
(12.) Desktop-Fenster-Frontend (z. B. tauri/egui) auf der `core`-Facade.
