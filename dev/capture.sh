#!/usr/bin/env bash
# Sichert eine Live-Seite als Entwicklungs-Fixture, unabhängig vom System-DNS.
# Löst den Host über Cloudflare-DoH (1.1.1.1) auf und holt die Seite mit
# Browser-User-Agent, folgt Redirects. Für die Resolver-Entwicklung an echten
# Hoster-/Portal-Seiten — Ausgabe landet in dev-fixtures/ (gitignored).
#
# Nutzung: dev/capture.sh <url> <ziel-datei>
set -euo pipefail
URL="${1:?url fehlt}"; OUT="${2:?ziel-datei fehlt}"
UA="Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36"
host=$(printf '%s' "$URL" | sed -E 's#^https?://([^/]+).*#\1#')
ip=$(curl -s -H 'accept: application/dns-json' "https://1.1.1.1/dns-query?name=${host}&type=A" \
     | python3 -c 'import sys,json;d=json.load(sys.stdin);print(next((a["data"] for a in d.get("Answer",[]) if a.get("type")==1),""))')
[ -z "$ip" ] && { echo "DNS-Auflösung für $host fehlgeschlagen" >&2; exit 1; }
mkdir -p "$(dirname "$OUT")"
curl -s -L --max-redirs 6 --resolve "${host}:443:${ip}" --resolve "${host}:80:${ip}" -A "$UA" "$URL" -o "$OUT"
printf 'gesichert: %s  (%s, %s Bytes, host=%s ip=%s)\n' "$OUT" "$URL" "$(wc -c <"$OUT")" "$host" "$ip"
