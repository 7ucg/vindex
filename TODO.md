# TODO

Offene Arbeit an vindex, grob nach Priorität. Hintergrund und Entwurfsgründe stehen in
[ARCHITECTURE.md](ARCHITECTURE.md); erledigte Punkte wandern ins
[CHANGELOG.md](CHANGELOG.md).

## Als Nächstes

- **Redirect-Hops revalidieren.** `validate_source_url` prüft nur die Einstiegs-URL;
  der reqwest-Client folgt Redirects danach ungeprüft. Eine `redirect::Policy::custom`
  soll jeden Hop gegen Schema (https), Host und private/loopback/link-local IP-Ranges
  prüfen, bevor er gefolgt wird.
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
- **Nativer HLS-Downloader** ohne ffmpeg-Abhängigkeit.
- **Watch** via mpv/IINA inkl. AniSkip, dazu Syncplay und Anime4K.

## Desktop

- **Fenster-Frontend** (tauri oder egui) auf der bestehenden `core`-Facade.
