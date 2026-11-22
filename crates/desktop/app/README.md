# vindex Desktop (Tauri + SolidJS)

Das Fenster-/Webview-Frontend auf der `vindex-desktop`-Facade. Spec:
[../DESKTOP-APP.md](../DESKTOP-APP.md), Bauplan: [../TODO.md](../TODO.md).

Eigenständiges Paket, **nicht** im Root-Cargo-Workspace (die GUI-Deps brauchen WebKitGTK;
der Workspace-Build soll webkit-frei bleiben).

## Voraussetzungen

- Rust (stable), Node ≥ 20, pnpm
- Linux: `webkit2gtk-4.1`, `libsoup-3.0`, GTK3 (Build-Pakete der Distro)
- `tauri-cli` v2 (`cargo install tauri-cli` oder `pnpm dlx @tauri-apps/cli`)

## Entwickeln

```bash
cd crates/desktop/app
pnpm install
pnpm tauri dev      # startet Vite + das Tauri-Fenster
```

## Bauen

```bash
pnpm tauri build    # AppImage/.deb (Linux), NSIS (Windows)
```

## Aufbau

```
app/
├─ src/                # SolidJS-Frontend
│  ├─ lib/             # api.ts (Tauri-Bridge), store, Logo, icons
│  └─ views/           # Sidebar + Ansichten
└─ src-tauri/          # Rust: Tauri-Commands auf die Facade (kein Netz-Code hier)
```

Das Frontend ruft nie direkt ins Netz — jeder Zugriff läuft über die Tauri-Commands in
`src-tauri/src/lib.rs` in `vindex-core`.
