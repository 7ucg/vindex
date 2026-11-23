# binaries

Ablage für mitgelieferte Programme, die die App zur Laufzeit nutzt, ohne dass man sie
systemweit installieren muss. Beim Start sucht die App hier (und neben der fertigen
Exe bzw. in `resources/binaries`) nach:

- `ffmpeg` — Remux/Mux, falls der ffmpeg-freie Pfad nicht greift
- `ffprobe` — Höhenprüfung bei Einzel-MP4
- `obscura` — Headless-Browser (Anti-Bot Stufe 3)

Unter Windows mit `.exe`. Wird ein Programm hier gefunden, setzt die App die passende
`VINDEX_*`-Variable automatisch darauf; fehlt es, fällt sie auf den PATH zurück. Das
gilt auch im Dev-Lauf (`tauri dev`) — es reicht, die Datei hier abzulegen.

Die Binärdateien liegen bewusst **nicht** im Repo (Größe/Lizenz). Lokal einfach
hineinkopieren, z. B. das System-ffmpeg:

```
cp "$(command -v ffmpeg)" ffmpeg
cp "$(command -v ffprobe)" ffprobe
# obscura: aus den eigenen Builds hierher kopieren
```

Für einen echten Release-Build gehören sie zusätzlich in die `resources` der
`tauri.conf.json`, damit sie neben die Exe wandern.
