# TODO

Offene Arbeit an vindex, grob nach Priorität. Hintergrund und Entwurfsgründe stehen in
[ARCHITECTURE.md](ARCHITECTURE.md); erledigte Punkte wandern ins
[CHANGELOG.md](CHANGELOG.md).

## Als Nächstes

- **serienstream.to-Scraper.** Die Seite wurde umgebaut und weicht von aniworld.to ab
  (live analysiert, Fixture in `dev-fixtures/serienstream-got-s1e1.html`):
  - Pfad `/serie/<slug>/staffel-X/episode-Y`, 301 von `/serie/stream/…` auf
    `http://…/serie/…`.
  - Hoster pro Element über `data-provider-name` (Name), `data-link-id` (Id) und
    `data-play-url="/r?t=<token>"`; Sprache über `data-language-id` (1=Deutsch,
    2=Englisch) und `data-language-label`.
  - `<token>` ist base64-JSON `{iv,value,mac,tag}` = Laravel-`Crypt` (AES-256-CBC+HMAC).
    Nicht clientseitig entschlüsseln: `GET https://serienstream.to/r?t=<token>` folgt
    serverseitig zum Hoster-Embed (wie aniworlds `/redirect/N`), den dann der normale
    Hoster-Resolver übernimmt.
  - Blocker: häufige Cloudflare-Captchas → braucht zusätzlich TLS-Impersonate/Browser
    (Stufe 2/3). Der Parser lässt sich aber schon gegen das Fixture bauen und testen.
  - s.to ist tot (Mirror s.to → serienstream.to bleibt sinnvoll).
- **Fehlende Hoster-Resolver.** Streamtape, Luluvdo und LoadX haben noch keinen Extraktor.
  Keine fertige Vorlage; Baumuster (siehe auch die Referenz unter `Source-Third-projects/`):
  - **Streamtape** wie Doodstream: Embed-HTML holen, `id="robotlink"`/`ideoolink`-innerHTML
    + nachfolgenden `substring`-Teil zusammensetzen, `https:` davor, `&stream=1` anhängen.
  - **Luluvdo** wie Filemoon: `/e/<code>` oder `/d/<code>` → packed JS → `unpack_js`
    (`hoster::common`) → `sources:[{file:"…m3u8"}]`; neuere Varianten über JSON-API (AES-GCM).
  - **LoadX** wie Vidstream: XHR-`getVideo`/`getSources` mit `X-Requested-With`, Antwort
    `m3u8`/`securedLink`, ggf. AES-CBC.
  Exakte Keys/Regexe müssen gegen Live-Embeds bestimmt werden — Embeds mit `dev/capture.sh`
  sichern, sobald ein Portal diese Hoster anbietet.

## Netzwerk & Anti-Bot

- **TLS-Impersonate (Stufe 2).** Bei Cloudflare-Verdacht den Abruf über `wreq` mit
  Chrome-Fingerprint wiederholen. Erkennung steht schon in `core::http`.
- **Headless-Browser-Fallback (Stufe 3).** Letzter Ausweg, wenn Fingerprinting nicht
  reicht.
- **Proxy-Rotation** für geratelimitete CDNs.

## Hoster-Pflege

- **Filemoon Byse-API.** Moderner Pfad mit AES-256-GCM; aktuell nur der Legacy-packed-JS-
  Weg. Muster liegt in der Referenz (`extractors/provider/filemoon.py`).

## API

- **Persistenter Key-Store.** Statt statischer Keys aus der Umgebung: Keys mit sha256
  speichern, Scopes (read/write/admin) an Methode und Route binden, Login-/Request-
  Throttle mit `429` und `Retry-After`.
- **Weitere Flächen.** Library, Auto-Sync, OpenAPI (`utoipa`), ein Rate-Limit-Layer
  (`tower_governor`).

## Download & Wiedergabe

- **Batch-Download** über eine Episodenliste und ein „keep watching".
- **Vollständig ffmpeg-freies HLS.** Der Segment-Download (`VINDEX_NATIVE_HLS`) läuft
  nativ, das Remuxen der zusammengesetzten Datei nutzt aber noch ffmpeg. Ein eigener
  TS-Remuxer würde ffmpeg ganz ersetzen; dazu AES-128-Entschlüsselung für verschlüsselte
  Playlists (fallen aktuell auf ffmpeg zurück).
- **Watch** via mpv/IINA inkl. AniSkip, dazu Syncplay und Anime4K.

## Entwicklung, Tests & Methoden

- **Fixture-basierte Scraper-/Resolver-Tests.** Reine Parse-Funktionen gegen gesicherte
  HTML-Beispiele testen (synthetische, token-freie Fixtures im Repo; echte Live-Seiten
  nur lokal in `dev-fixtures/`, gitignored). Deckt die Scraper ab, ohne Netz und ohne
  Portal-Inhalte zu veröffentlichen.
- **Capture-Methode** (`dev/capture.sh <url> <datei>`): sichert eine Live-Seite über
  Cloudflare-DoH (1.1.1.1) mit Browser-UA nach `dev-fixtures/`. Grundlage, um neue
  Hoster-/Portal-Varianten offline zu analysieren.
- **Integrationstests gegen den Daemon** (axum-`oneshot`): Queue-Limit (429), Gesamt-Cap
  und Statusübergänge sind noch ungetestet.
- **Behobene Beobachtungen bewerten:** Suche ist reiner Substring-Abgleich (Punkt zählt);
  `--quality` deckelt Single-MP4-Hoster nicht nach oben; `naming::clean_title` schreibt
  klein. Jeweils entscheiden, ob gewünscht.

## Desktop

- **Facade vervollständigen.** `Backend` reicht bisher nur `search` und `providers`
  durch. Vor einem GUI fehlen: Download (`download_episode`/`download_url` + `DownloadPrefs`)
  mit Fortschritts-Reporting, Zugriff auf `config::Settings`, und die Hoster→Stream-Auflösung.
  Dazu mindestens ein headless-Test (die Doku in `lib.rs` behauptet „testbar", 0 Tests).
- **Fenster-Frontend** (tauri oder egui) auf der vervollständigten `core`-Facade.
