# TODO

Nur offene Arbeit. Erledigtes steht im [CHANGELOG.md](CHANGELOG.md), Hintergrund in
[ARCHITECTURE.md](ARCHITECTURE.md).

## Offene Features

- **Desktop-Fenster-Frontend** (tauri oder egui) auf der `core`-Facade. Facade und
  Fortschritts-Kanal (`core::download::Progress`/`ProgressSink`, `Backend::download_with_progress`)
  sind da; offen ist nur das Fenster selbst (Event-Loop + Widgets), das wegen der System-
  Bibliotheken (WebKitGTK bzw. GPU) bewusst außerhalb des Workspaces liegt.
- **Echter TS→MKV-Remuxer ohne ffmpeg** (optional). `VINDEX_NO_FFMPEG` liefert bereits einen
  rohen `.ts`-Stream ohne ffmpeg; ein eigener Remuxer für `.mkv`-Container wäre die Kür.
  `SAMPLE-AES` bliebe am ffmpeg-Pfad. Braucht echte TS-Segmente zum Verifizieren.

## Betrieb

- **Produktions-Rate-Limit.** Das eingebaute Limit ist Best-Effort im Prozessspeicher; für
  öffentlichen Betrieb gehört ein Edge-/Reverse-Proxy-Limit davor (verteilter Limiter mit
  Trusted-Proxy-`X-Forwarded-For`). Der In-Memory-Teil bleibt Defense-in-Depth.

## Nur in deiner Umgebung zu bestätigen (Code steht, Logik getestet)

- **Mehrstaffel-MAL-ID** (`aniskip::resolve_mal_id`): folgt der echten Sequel-Kette; der Live-
  Abruf gegen Jikan war aus der Testumgebung egress-geblockt.
- **Syncplay** (`--syncplay`): Arg-/Config-Bau abgesichert und getestet; der Lauf gegen einen
  echten Syncplay-Server/-Raum ist nur mit installiertem Syncplay prüfbar.
- **Neue Hoster**, falls ein Portal sie wieder listet (z. B. Streamtape): Embed mit
  `dev/capture.sh` sichern, dann Extraktor bauen.

## Produkt-Entscheidungen (Default: so lassen)

- Suche ist reiner Substring (Punkt zählt); `--quality` deckelt Single-MP4-Hoster nicht nach
  oben; `naming::clean_title` schreibt klein.
