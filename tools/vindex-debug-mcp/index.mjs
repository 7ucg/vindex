#!/usr/bin/env node
// MCP-Server zum Debuggen der vindex-Desktop-App.
//
// Werkzeuge:
//   resolve  – eine Episoden-/Film-URL headless zum Stream auflösen (ruft die
//              vindex-CLI: `vindex watch <url> --print-url`). Zeigt, ob Scraping
//              und Hoster-Auflösung funktionieren, ohne die GUI.
//   probe    – eine (Stream-)URL abrufen und Status/Content-Type/Anfang melden;
//              so wie der Referer-Proxy der App es täte. Zeigt, ob das Ziel
//              erreichbar ist und eine gültige Playlist/Datei liefert.
//   search   – den aniworld-Katalog nach einem Stichwort durchsuchen.
//
// Die vindex-CLI wird über $VINDEX_BIN gefunden, sonst im üblichen target/-Pfad.

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { Resolver } from "node:dns/promises";
import { isIP } from "node:net";

// DNS fest über 1.1.1.1 (nie System-DNS) — Barons „1.1.1.1 überall"-Regel.
const dns1111 = new Resolver();
dns1111.setServers(["1.1.1.1", "[2606:4700:4700::1111]"]);

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "..", "..");
const UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

function vindexBin() {
  if (process.env.VINDEX_BIN) return process.env.VINDEX_BIN;
  for (const p of ["target/release/vindex", "target/debug/vindex"]) {
    const full = join(REPO, p);
    if (existsSync(full)) return full;
  }
  return "vindex"; // auf PATH hoffen
}

function run(bin, args, timeoutMs = 120000) {
  return new Promise((resolve) => {
    execFile(bin, args, { timeout: timeoutMs, maxBuffer: 8 * 1024 * 1024 }, (err, stdout, stderr) => {
      resolve({ code: err?.code ?? 0, stdout: stdout ?? "", stderr: stderr ?? "", error: err?.message });
    });
  });
}

async function toolResolve({ url, provider, language }) {
  const args = ["watch", url, "--print-url"];
  if (provider) args.push("--provider", provider);
  if (language) args.push("--language", language);
  const r = await run(vindexBin(), args);
  // Die JSON-Zeile steht am Ende der Ausgabe.
  const line = r.stdout.trim().split("\n").filter(Boolean).pop() ?? "";
  let parsed = null;
  try {
    parsed = JSON.parse(line);
  } catch {
    /* keine JSON-Zeile → Fehlertext steht in stderr/stdout */
  }
  return {
    resolved: parsed,
    exit_code: r.code,
    stderr: r.stderr.trim() || undefined,
    raw: parsed ? undefined : r.stdout.trim(),
  };
}

// Private/lokale/reservierte IP? (v4 inkl. IPv4-mapped-v6, CGNAT, link-local …)
function isPrivateIp(ip) {
  let a = ip;
  const m = /^::ffff:(\d+\.\d+\.\d+\.\d+)$/i.exec(ip);
  if (m) a = m[1];
  if (isIP(a) === 4) {
    const o = a.split(".").map(Number);
    return (
      o[0] === 0 ||
      o[0] === 10 ||
      o[0] === 127 ||
      (o[0] === 100 && o[1] >= 64 && o[1] <= 127) || // CGNAT 100.64/10
      (o[0] === 169 && o[1] === 254) || // link-local
      (o[0] === 172 && o[1] >= 16 && o[1] <= 31) ||
      (o[0] === 192 && o[1] === 168) ||
      (o[0] === 198 && (o[1] === 18 || o[1] === 19)) || // benchmarking
      o[0] >= 224 // multicast/reserved
    );
  }
  const l = a.toLowerCase();
  return (
    l === "::1" ||
    l === "::" ||
    l.startsWith("fc") ||
    l.startsWith("fd") || // unique local
    l.startsWith("fe80") // link-local
  );
}

async function hostAllowed(host) {
  const bare = host.replace(/^\[|\]$/g, "");
  if (["localhost", "metadata", "metadata.google.internal"].includes(bare.toLowerCase())) {
    return false;
  }
  if (isIP(bare)) return !isPrivateIp(bare);
  try {
    const [a4, a6] = await Promise.all([
      dns1111.resolve4(bare).catch(() => []),
      dns1111.resolve6(bare).catch(() => []),
    ]);
    const addrs = [...a4, ...a6];
    return addrs.length > 0 && addrs.every((ip) => !isPrivateIp(ip));
  } catch {
    return false;
  }
}

async function toolProbe({ url, referer }) {
  const headers = { "User-Agent": UA };
  if (referer) headers["Referer"] = referer;
  const started = Date.now();
  try {
    // SSRF-Bremse: wie der App-Proxy. http/https, keine privaten/lokalen Ziele,
    // Redirects manuell und pro Hop neu geprüft (max. 5).
    let current = new URL(url);
    let res;
    for (let hop = 0; hop <= 5; hop++) {
      if (!/^https?:$/.test(current.protocol)) return { error: "nur http/https erlaubt" };
      if (!(await hostAllowed(current.hostname))) return { error: `Ziel blockiert: ${current.hostname}` };
      const ctrl = new AbortController();
      const t = setTimeout(() => ctrl.abort(), 30000);
      res = await fetch(current, { headers, redirect: "manual", signal: ctrl.signal });
      clearTimeout(t);
      if (res.status >= 300 && res.status < 400 && res.headers.get("location")) {
        current = new URL(res.headers.get("location"), current);
        continue;
      }
      break;
    }
    const buf = Buffer.from(await res.arrayBuffer());
    const ct = res.headers.get("content-type") || "";
    const isText = /mpegurl|text|json|xml/i.test(ct);
    return {
      status: res.status,
      status_text: res.statusText,
      content_type: ct,
      bytes: buf.length,
      ms: Date.now() - started,
      final_url: current.toString(),
      head: isText ? buf.subarray(0, 600).toString("utf8") : `<${buf.length} bytes binär>`,
    };
  } catch (e) {
    return { error: String(e?.message ?? e), ms: Date.now() - started };
  }
}

async function toolSearch({ keyword }) {
  const r = await run(vindexBin(), ["search", keyword], 90000);
  return { exit_code: r.code, output: r.stdout.trim(), stderr: r.stderr.trim() || undefined };
}

const TOOLS = [
  {
    name: "resolve",
    description:
      "Löst eine aniworld/serienstream/filmo Episoden- oder Film-URL headless zum Stream auf (vindex watch --print-url). Gibt {url,referer,is_hls} oder den Fehler zurück.",
    inputSchema: {
      type: "object",
      properties: {
        url: { type: "string", description: "Episoden-/Film-URL" },
        provider: { type: "string", description: "Hoster erzwingen (VOE, Vidoza, Filemoon, …)" },
        language: { type: "string", description: "Sprache (German Dub, German Sub, …)" },
      },
      required: ["url"],
    },
  },
  {
    name: "probe",
    description:
      "Ruft eine (Stream-)URL ab und meldet Status, Content-Type, Größe und den Anfang — wie der Referer-Proxy der App. Zeigt, ob das Ziel erreichbar ist.",
    inputSchema: {
      type: "object",
      properties: {
        url: { type: "string", description: "Abzurufende URL (z. B. die aufgelöste m3u8)" },
        referer: { type: "string", description: "Optionaler Referer-Header" },
      },
      required: ["url"],
    },
  },
  {
    name: "search",
    description: "Durchsucht den aniworld-Katalog nach einem Stichwort (vindex search).",
    inputSchema: {
      type: "object",
      properties: { keyword: { type: "string" } },
      required: ["keyword"],
    },
  },
];

const HANDLERS = { resolve: toolResolve, probe: toolProbe, search: toolSearch };

const server = new Server(
  { name: "vindex-debug-mcp", version: "0.1.0" },
  { capabilities: { tools: {} } },
);

server.setRequestHandler(ListToolsRequestSchema, async () => ({ tools: TOOLS }));

server.setRequestHandler(CallToolRequestSchema, async (req) => {
  const handler = HANDLERS[req.params.name];
  if (!handler) {
    return { content: [{ type: "text", text: `Unbekanntes Werkzeug: ${req.params.name}` }], isError: true };
  }
  try {
    const result = await handler(req.params.arguments ?? {});
    return { content: [{ type: "text", text: JSON.stringify(result, null, 2) }] };
  } catch (e) {
    return { content: [{ type: "text", text: `Fehler: ${String(e?.message ?? e)}` }], isError: true };
  }
});

await server.connect(new StdioServerTransport());
console.error("vindex-debug-mcp bereit (stdio)");
