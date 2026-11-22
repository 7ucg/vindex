# vindex-debug-mcp

MCP-Server zum Debuggen der vindex-Desktop-App — ohne die GUI zu starten.

## Werkzeuge

- **resolve** `{ url, provider?, language? }` — löst eine Episoden-/Film-URL headless zum Stream auf
  (ruft `vindex watch <url> --print-url`). Zeigt `{url, referer, is_hls}` oder den Fehler. Damit
  prüfst du, ob Scraping und Hoster-Auflösung sitzen.
- **probe** `{ url, referer? }` — ruft eine URL ab und meldet Status, Content-Type, Größe und den
  Anfang, genau wie der Referer-Proxy der App. Zeigt, ob der Stream erreichbar ist und eine
  gültige Playlist liefert.
- **search** `{ keyword }` — durchsucht den aniworld-Katalog.

## Installieren

```bash
cd tools/vindex-debug-mcp
npm install
# Die vindex-CLI muss gebaut sein (der Server findet target/debug/vindex automatisch):
cargo build -p vindex
```

Der Server sucht die CLI über `$VINDEX_BIN`, sonst in `target/{release,debug}/vindex` des Repos,
sonst auf dem `PATH`.

## In Claude Code registrieren

```bash
claude mcp add vindex-debug -- node /home/baron/Dokumente/GitHub/vindex/tools/vindex-debug-mcp/index.mjs
```

Oder in der MCP-Konfiguration:

```json
{
  "mcpServers": {
    "vindex-debug": {
      "command": "node",
      "args": ["/home/baron/Dokumente/GitHub/vindex/tools/vindex-debug-mcp/index.mjs"]
    }
  }
}
```

Nach dem Registrieren eine neue Session starten, dann stehen `resolve`, `probe` und `search`
als Tools bereit.

## Typischer Debug-Ablauf

1. `resolve` mit der Episoden-URL → bekomme die Stream-URL + Referer.
2. `probe` mit genau dieser Stream-URL (und Referer) → sehe Status/Content-Type.
   - 200 + `application/vnd.apple.mpegurl` → Stream ist gut, Fehler liegt im Proxy/Player.
   - 403/andere → Hoster blockt (Impersonation/Referer nötig).
