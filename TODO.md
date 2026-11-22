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

- **DNS-Rebinding.** Die Redirect-Policy sperrt IP-Literale in privaten Ranges, aber
  kein Hostname, der erst per DNS auf eine private IP zeigt. Dafür bräuchte es einen
  eigenen Resolver/Connector, der die aufgelöste IP vor dem Connect prüft.
- **Transient-Retry.** Der Referenz-Client wiederholt bei 429/5xx mit Backoff; vindex
  hat `with_backoff`, nutzt es für die Scrape-/Resolve-Fetches aber noch nicht.
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
