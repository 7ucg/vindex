# TODO

Offene Arbeit an vindex, grob nach Priorität. Hintergrund und Entwurfsgründe stehen in
[ARCHITECTURE.md](ARCHITECTURE.md); erledigte Punkte wandern ins
[CHANGELOG.md](CHANGELOG.md).

## Als Nächstes

- **Fehlende Hoster-Resolver.** Streamtape, Luluvdo und LoadX haben noch keinen
  Extraktor. Keine Vorlage zum Abschreiben; die Algorithmen müssen live ermittelt
  werden. Baumuster: Streamtape wie Doodstream (Token aus HTML + Referer), Luluvdo wie
  Filemoon (packed JS / Byse-AES-GCM), LoadX wie ein XHR-getSources-Endpunkt.

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

## Desktop

- **Facade vervollständigen.** `Backend` reicht bisher nur `search` und `providers`
  durch. Vor einem GUI fehlen: Download (`download_episode`/`download_url` + `DownloadPrefs`)
  mit Fortschritts-Reporting, Zugriff auf `config::Settings`, und die Hoster→Stream-Auflösung.
  Dazu mindestens ein headless-Test (die Doku in `lib.rs` behauptet „testbar", 0 Tests).
- **Fenster-Frontend** (tauri oder egui) auf der vervollständigten `core`-Facade.
