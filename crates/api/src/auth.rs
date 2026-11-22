//! API-Key-Authentifizierung mit Scopes und persistentem Key-Store.
//!
//! Schlüssel kommen per Header `X-API-Key` oder `Authorization: Bearer <key>`.
//! Es gibt zwei Quellen:
//!
//! * **Bootstrap (Env):** `VINDEX_API_KEYS` (Klartext, komma-getrennt). Diese
//!   Keys bekommen den Scope [`Scope::Admin`] (deckt alles) und bleiben aus
//!   Rückwärtskompatibilität gültig.
//! * **Persistenter Store (Datei):** JSON-Liste aus `{ "hash", "scope", "name" }`
//!   unter dem Pfad `VINDEX_API_KEYS_FILE` (Default: `~/.config/vindex/api_keys.json`).
//!   Gespeichert wird **nur der sha256-Hash** des Keys, nie der Klartext.
//!
//! Beim Request wird der präsentierte Key sha256t und konstant gegen die
//! hinterlegten Hashes verglichen. Jede Route verlangt einen Mindest-Scope
//! (siehe [`required_scope`]); ein höherer Scope deckt die niedrigeren ab
//! (Admin ⊇ Write ⊇ Read). Reicht der Scope nicht, antwortet die Middleware mit
//! `403`; ein fehlender/ungültiger Key weiterhin mit `401`.
//!
//! Ohne jede konfigurierte Quelle ist die API offen — dann lässt [`crate::serve`]
//! aber nur einen localhost-Bind zu.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

/// Zugriffs-Scope eines Keys. Geordnet: Read < Write < Admin. Ein Key mit
/// höherem Scope deckt die niedrigeren ab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Lesende Endpunkte (GET).
    Read,
    /// Schreibende Endpunkte (z. B. `POST /api/download`).
    Write,
    /// Volle Rechte; deckt alles ab und steuert die `/api/keys`-Verwaltung.
    Admin,
}

impl Scope {
    /// Rang für die Hierarchie-Prüfung (höher = mehr Rechte).
    fn rank(self) -> u8 {
        match self {
            Scope::Read => 0,
            Scope::Write => 1,
            Scope::Admin => 2,
        }
    }

    /// True, wenn dieser Scope den geforderten abdeckt (Admin ⊇ Write ⊇ Read).
    fn covers(self, needed: Scope) -> bool {
        self.rank() >= needed.rank()
    }

    /// Scope als Kleinbuchstaben-String (`read` | `write` | `admin`) — für die
    /// JSON-Antworten der Key-Verwaltung und den Store (nie der Hash).
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "read",
            Scope::Write => "write",
            Scope::Admin => "admin",
        }
    }
}

/// Ein hinterlegter Key: sha256-Hash (hex, lowercase), Scope und ein Name zur
/// Wiedererkennung. Der Klartext wird nie gehalten.
#[derive(Clone)]
struct KeyEntry {
    hash: String,
    scope: Scope,
    name: String,
}

/// Rohform eines Store-Eintrags beim JSON-Parsen.
#[derive(Deserialize)]
struct StoredKey {
    /// sha256-Hex des Keys.
    hash: String,
    /// Scope: `read` | `write` | `admin`.
    scope: Scope,
    /// Freier Name/Label (optional).
    #[serde(default)]
    name: Option<String>,
}

/// Ergebnis der Autorisierung einer Anfrage.
enum Decision {
    /// Durchlassen (gültiger Key mit ausreichendem Scope, oder offene API).
    Allow,
    /// Fehlender oder unbekannter Key → `401`.
    Unauthorized,
    /// Gültiger Key, aber Scope zu niedrig → `403`.
    Forbidden,
}

/// Konfigurierte API-Keys. Geklont pro Request (nur zwei `Arc`-Bumps).
///
/// Zwei Quellen werden getrennt gehalten, damit ein Hot-Reload nur den
/// datei-basierten Teil (`stored`) ersetzt und die Env-Bootstrap-Keys
/// (`bootstrap`) unverändert bestehen bleiben — die stehen nie in der Datei.
#[derive(Clone)]
pub struct AuthConfig {
    /// Env-Bootstrap-Keys (`VINDEX_API_KEYS`), stets [`Scope::Admin`]. Nicht in
    /// der Store-Datei, daher über `/api/keys` weder listbar noch löschbar.
    bootstrap: Arc<Vec<KeyEntry>>,
    /// Aus der Store-Datei geladene Einträge. Wird beim Hot-Reload ersetzt.
    stored: Arc<Vec<KeyEntry>>,
    /// Konnte ein explizit gesetzter/vorhandener Store nicht geladen werden, ist
    /// die Auth gesperrt (deny-all) statt offen — kein Fail-open bei Konfig-Fehler.
    locked: bool,
    /// Startete der Daemon ganz ohne Keys (offener Modus)? Nur dann bleibt die API
    /// offen. Werden zur Laufzeit alle Keys gelöscht, wird gesperrt, nicht geöffnet.
    was_open_at_start: bool,
}

impl AuthConfig {
    /// Baut die Konfiguration aus Bootstrap-Env-Keys und bereits geparsten
    /// Store-Einträgen. Env-Keys bekommen [`Scope::Admin`].
    fn build(env_keys: &[String], stored: Vec<KeyEntry>, locked: bool) -> Self {
        let bootstrap: Vec<KeyEntry> = env_keys
            .iter()
            .filter(|k| !k.is_empty())
            .map(|k| KeyEntry {
                hash: sha256_hex(k),
                scope: Scope::Admin,
                name: "env-bootstrap".to_string(),
            })
            .collect();
        let was_open_at_start = !locked && bootstrap.is_empty() && stored.is_empty();
        AuthConfig {
            bootstrap: Arc::new(bootstrap),
            stored: Arc::new(stored),
            locked,
            was_open_at_start,
        }
    }

    /// Übernimmt die Bootstrap-Keys aus den Settings und lädt zusätzlich den
    /// persistenten Store (Datei).
    pub fn from_settings(s: &vindex_core::config::Settings) -> Self {
        let (stored, load_failed) = load_store();
        if !stored.is_empty() {
            let names: Vec<&str> = stored.iter().map(|e| e.name.as_str()).collect();
            tracing::info!(
                "API-Key-Store: {} Schlüssel geladen ({})",
                stored.len(),
                names.join(", ")
            );
        }
        AuthConfig::build(&s.api_keys, stored, load_failed)
    }

    /// Lädt den persistenten Store erneut aus der Datei und gibt eine neue
    /// Konfiguration mit frischem `stored`-Teil zurück. Die Env-Bootstrap-Keys
    /// bleiben unverändert. So wirken über `/api/keys` angelegte/gelöschte Keys
    /// sofort, ohne den Daemon neu zu starten.
    pub fn reloaded(&self) -> AuthConfig {
        let (stored, load_failed) = load_store();
        // Ein leerer Key-Satz nach einem Reload (z. B. letzter Key gelöscht) ist ein
        // Konfigurationsfehler, kein offener Modus — offen bleibt nur, was ganz ohne
        // Keys gestartet ist.
        // „Offen" gilt nur, solange noch nie Keys existierten; sobald einmal welche da
        // waren, verfällt das Flag — das Löschen des letzten Keys sperrt dann.
        let still_open =
            self.was_open_at_start && self.bootstrap.is_empty() && self.stored.is_empty();
        let runtime_empty = !still_open && self.bootstrap.is_empty() && stored.is_empty();
        AuthConfig {
            bootstrap: self.bootstrap.clone(),
            stored: Arc::new(stored),
            locked: load_failed || runtime_empty,
            was_open_at_start: still_open,
        }
    }

    /// Listet die Einträge des persistenten Stores als `(name, scope)` — ohne
    /// Hash und ohne Klartext, und ohne die nicht verwaltbaren Env-Bootstrap-Keys.
    pub fn stored_keys(&self) -> Vec<(String, Scope)> {
        self.stored
            .iter()
            .map(|e| (e.name.clone(), e.scope))
            .collect()
    }

    /// Test-Konstruktor mit festen Klartext-Keys (Admin-Scope, wie Bootstrap).
    #[cfg(test)]
    pub(crate) fn with_keys(keys: &[&str]) -> Self {
        let env: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        AuthConfig::build(&env, Vec::new(), false)
    }

    /// Test-Konstruktor mit Klartext-Keys und explizitem Scope (Store-artig).
    #[cfg(test)]
    pub(crate) fn with_scoped(keys: &[(&str, Scope)]) -> Self {
        let stored = keys
            .iter()
            .map(|(k, scope)| KeyEntry {
                hash: sha256_hex(k),
                scope: *scope,
                name: "test".to_string(),
            })
            .collect();
        AuthConfig::build(&[], stored, false)
    }

    /// True, wenn kein Key konfiguriert ist UND der Store nicht fehlerhaft war
    /// (dann ist die API offen). Bei Lade-Fehler ist sie gesperrt, nicht offen.
    pub fn is_open(&self) -> bool {
        self.bootstrap.is_empty() && self.stored.is_empty() && !self.locked
    }

    /// Prüft die Request-Header gegen die hinterlegten Keys und den geforderten
    /// Scope.
    fn authorize(&self, headers: &HeaderMap, required: Scope) -> Decision {
        if self.bootstrap.is_empty() && self.stored.is_empty() {
            // Kein Key konfiguriert → offen; ein fehlgeschlagener Store-Ladevorgang
            // sperrt dagegen alles (deny-all), statt die API versehentlich zu öffnen.
            if self.locked {
                return Decision::Unauthorized;
            }
            // Admin-Routen (Key-Verwaltung) bleiben auch im offenen Modus zu: sonst
            // könnte unauthentifiziert ein Key angelegt werden. Erst ein echter
            // Admin-Key (per VINDEX_API_KEYS-Bootstrap) macht die Verwaltung nutzbar.
            if required == Scope::Admin {
                return Decision::Unauthorized;
            }
            return Decision::Allow;
        }
        let provided = match extract_key(headers) {
            Some(p) => p,
            None => return Decision::Unauthorized,
        };
        let hash = sha256_hex(&provided);
        // Alle Einträge (Bootstrap + Store) durchgehen (kein früher Abbruch) und den
        // höchsten passenden Scope sammeln. ct_eq vergleicht die Hex-Hashes längengebunden.
        let mut best: Option<Scope> = None;
        for e in self.bootstrap.iter().chain(self.stored.iter()) {
            if ct_eq(&e.hash, &hash) {
                best = Some(match best {
                    Some(s) if s.rank() >= e.scope.rank() => s,
                    _ => e.scope,
                });
            }
        }
        match best {
            None => Decision::Unauthorized,
            Some(s) if s.covers(required) => Decision::Allow,
            Some(_) => Decision::Forbidden,
        }
    }
}

/// Mindest-Scope für eine Route/Methode. Die Key-Verwaltung unter `/api/keys`
/// verlangt [`Scope::Admin`], `POST /api/download` [`Scope::Write`], alle übrigen
/// (GET-)Endpunkte [`Scope::Read`].
fn required_scope(method: &Method, path: &str) -> Scope {
    if path == "/api/keys" || path.starts_with("/api/keys/") {
        Scope::Admin
    } else if method == Method::POST && path == "/api/download" {
        Scope::Write
    } else {
        Scope::Read
    }
}

/// Zieht den Key aus `X-API-Key` oder `Authorization: Bearer <key>`.
fn extract_key(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    let auth = headers.get("authorization")?.to_str().ok()?;
    let token = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// sha256-Hash eines Strings als Hex (lowercase, 64 Zeichen).
///
/// Bewusst ein schneller Hash, kein langsamer KDF (argon2/scrypt): die Keys sind
/// hochentropisch (256-Bit-Zufall aus [`generate_key`]), da ist ein Offline-
/// Bruteforce nicht praktikabel und ein Password-KDF wäre fehl am Platz. Keys
/// sollten daher ausschließlich aus `generate_key` stammen, nicht aus Passwörtern.
fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    out
}

/// Längengebundener, konstant laufender Vergleich (vermeidet frühen Abbruch bei
/// gleicher Länge). Die Länge selbst wird nicht verschleiert.
fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Pfad des persistenten Key-Stores: `VINDEX_API_KEYS_FILE` oder der Default
/// neben der Config (`~/.config/vindex/api_keys.json`). Das `bool` meldet, ob
/// der Pfad explizit per Env gesetzt wurde (dann ist eine fehlende Datei ein
/// Konfigurationsfehler und wird geloggt).
fn store_path() -> Option<(PathBuf, bool)> {
    match std::env::var_os("VINDEX_API_KEYS_FILE") {
        Some(p) if !p.is_empty() => Some((PathBuf::from(p), true)),
        _ => default_store_path().map(|p| (p, false)),
    }
}

/// Default-Pfad des Stores: `$HOME/.config/vindex/api_keys.json`.
fn default_store_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| {
        PathBuf::from(h)
            .join(".config")
            .join("vindex")
            .join("api_keys.json")
    })
}

/// Lädt und parst den persistenten Key-Store. Fehlt die Datei (Default-Pfad),
/// ist das kein Fehler → leere Liste. Ein kaputter Inhalt oder eine fehlende
/// explizit gesetzte Datei wird geloggt und ergibt ebenfalls eine leere Liste
/// (die API fällt dann auf die Env-Bootstrap-Keys bzw. den offenen Modus zurück).
fn load_store() -> (Vec<KeyEntry>, bool) {
    let (path, explicit) = match store_path() {
        Some(p) => p,
        None => return (Vec::new(), false),
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => match parse_store(&text) {
            Ok(entries) => (entries, false),
            Err(e) => {
                // Datei existiert, ist aber kein gültiges JSON-Array → Auth sperren,
                // nicht öffnen.
                tracing::error!(
                    "Key-Store {} ungültig ({e}) — Auth gesperrt (deny-all)",
                    path.display()
                );
                (Vec::new(), true)
            }
        },
        Err(e) => {
            if explicit {
                // Explizit gesetzte Datei nicht lesbar → sperren.
                tracing::error!(
                    "Key-Store {} nicht lesbar ({e}) — Auth gesperrt (deny-all)",
                    path.display()
                );
                (Vec::new(), true)
            } else {
                // Default-Datei fehlt einfach → kein Fehler, API wie bisher.
                (Vec::new(), false)
            }
        }
    }
}

/// Parst den JSON-Inhalt des Key-Stores zu [`KeyEntry`]-Einträgen. Der Top-Level
/// muss ein JSON-Array sein (sonst `Err` → Auth gesperrt); einzelne fehlerhafte
/// Einträge werden übersprungen, damit ein kaputter Datensatz nicht alle Keys
/// verwirft. Hashes werden getrimmt und kleingeschrieben.
fn parse_store(text: &str) -> Result<Vec<KeyEntry>, serde_json::Error> {
    let raw: Vec<serde_json::Value> = serde_json::from_str(text)?;
    let mut out = Vec::new();
    for v in raw {
        match serde_json::from_value::<StoredKey>(v) {
            Ok(s) => out.push(KeyEntry {
                hash: s.hash.trim().to_ascii_lowercase(),
                scope: s.scope,
                name: s.name.unwrap_or_default(),
            }),
            Err(e) => tracing::warn!("Key-Store: Eintrag übersprungen ({e})"),
        }
    }
    Ok(out)
}

/// Erzeugt einen neuen Klartext-Key und seinen sha256-Hash. Der Klartext ist
/// hex-kodierter CSPRNG-Zufall (32 Byte); im Store wird nur der Hash abgelegt.
/// Hilfe zum Befüllen des Stores — keine Verwaltungsroute. Schlägt die Zufalls-
/// quelle fehl, wird KEIN schwacher Key erzeugt, sondern ein Fehler zurückgegeben.
pub fn generate_key() -> Result<(String, String), getrandom::Error> {
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf)?;
    let mut plain = String::with_capacity(64);
    for b in buf {
        plain.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        plain.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    let hash = sha256_hex(&plain);
    Ok((plain, hash))
}

/// Fehler der Store-Verwaltung (Anlegen/Löschen von Keys über `/api/keys`).
#[derive(Debug)]
pub enum KeyMgmtError {
    /// Kein Store-Pfad bestimmbar (weder `VINDEX_API_KEYS_FILE` noch `HOME`).
    NoPath,
    /// Die vorhandene Store-Datei ist kein gültiges JSON-Array — nicht
    /// überschreiben, sondern melden (sonst gingen fremde Einträge verloren).
    Corrupt,
    /// Die Zufallsquelle für den neuen Key stand nicht bereit.
    Rng,
    /// Der Name ist leer, zu lang oder enthält unerlaubte Zeichen.
    InvalidName,
    /// Ein Key mit diesem Namen existiert bereits.
    Duplicate,
    /// Der Store hat die maximale Zahl an Einträgen erreicht.
    Full,
    /// Datei-/IO-Fehler beim Lesen oder atomaren Schreiben.
    Io(std::io::Error),
}

/// Obergrenze gespeicherter Keys und maximale Namenslänge.
const MAX_STORE_KEYS: usize = 1000;
const MAX_KEY_NAME_LEN: usize = 64;

/// True, wenn *name* ein gültiger Key-Name ist: nicht leer, höchstens
/// [`MAX_KEY_NAME_LEN`] Zeichen, nur `[A-Za-z0-9._-]`.
fn valid_key_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_KEY_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl std::fmt::Display for KeyMgmtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyMgmtError::NoPath => write!(
                f,
                "kein Key-Store-Pfad gesetzt (VINDEX_API_KEYS_FILE oder HOME nötig)"
            ),
            KeyMgmtError::Corrupt => {
                write!(f, "vorhandene Key-Store-Datei ist kein gültiges JSON-Array")
            }
            KeyMgmtError::Rng => write!(f, "Zufallsquelle für den neuen Key nicht verfügbar"),
            KeyMgmtError::InvalidName => write!(
                f,
                "ungültiger Key-Name (nicht leer, max. 64 Zeichen, nur A-Z a-z 0-9 . _ -)"
            ),
            KeyMgmtError::Duplicate => write!(f, "ein Key mit diesem Namen existiert bereits"),
            KeyMgmtError::Full => write!(f, "Key-Store ist voll"),
            KeyMgmtError::Io(e) => write!(f, "Key-Store-IO-Fehler: {e}"),
        }
    }
}

impl std::error::Error for KeyMgmtError {}

/// Hex-Kodierung eines Byte-Puffers (lowercase).
fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    out
}

/// Liest die aktuellen Einträge aus der Store-Datei (nur die Datei, nicht die
/// Env-Bootstrap-Keys). Eine fehlende Datei ergibt eine leere Liste; eine
/// vorhandene, aber kaputte Datei einen [`KeyMgmtError::Corrupt`] (damit sie
/// beim Anhängen nicht versehentlich überschrieben wird).
fn read_store_file() -> Result<(PathBuf, Vec<KeyEntry>), KeyMgmtError> {
    let (path, _) = store_path().ok_or(KeyMgmtError::NoPath)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let entries = parse_store(&text).map_err(|_| KeyMgmtError::Corrupt)?;
            Ok((path, entries))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((path, Vec::new())),
        Err(e) => Err(KeyMgmtError::Io(e)),
    }
}

/// Serialisiert die Store-Einträge als JSON-Array `[{hash,scope,name}]`.
fn serialize_entries(entries: &[KeyEntry]) -> String {
    let arr: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            json!({
                "hash": e.hash,
                "scope": e.scope.as_str(),
                "name": e.name,
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::Value::Array(arr))
        .unwrap_or_else(|_| "[]".to_string())
}

/// Schreibt die Store-Datei atomar und privat: Verzeichnis auf `0700`, die Daten
/// in eine frisch & exklusiv angelegte Temp-Datei (`create_new`/O_EXCL folgt
/// keinem Symlink) mit unvorhersagbarem Namen und Rechten `0600`, dann umbenennen.
fn write_store_atomic(path: &Path, entries: &[KeyEntry]) -> Result<(), KeyMgmtError> {
    use std::io::Write;

    let data = serialize_entries(entries);
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    if let Some(d) = dir {
        std::fs::create_dir_all(d).map_err(KeyMgmtError::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700));
        }
    }

    for _ in 0..8u32 {
        // Unvorhersagbarer Temp-Name aus CSPRNG-Zufall.
        let mut rnd = [0u8; 16];
        getrandom::getrandom(&mut rnd).map_err(|_| KeyMgmtError::Rng)?;
        let name = format!(".api_keys.{}.tmp", to_hex(&rnd));
        let tmp = match dir {
            Some(d) => d.join(&name),
            None => PathBuf::from(&name),
        };

        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = match opts.open(&tmp) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(KeyMgmtError::Io(e)),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
        }
        if let Err(e) = f.write_all(data.as_bytes()).and_then(|_| f.sync_all()) {
            drop(f);
            let _ = std::fs::remove_file(&tmp);
            return Err(KeyMgmtError::Io(e));
        }
        drop(f);
        return match std::fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(KeyMgmtError::Io(e))
            }
        };
    }
    Err(KeyMgmtError::Io(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "Temp-Datei für Key-Store konnte nicht angelegt werden",
    )))
}

/// Serialisiert alle Store-Mutationen (read-modify-write), damit zwei parallele
/// `/api/keys`-Requests sich nicht gegenseitig überschreiben (lost update).
fn store_mutation_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Legt einen neuen Key an: erzeugt den Klartext per CSPRNG, hängt nur
/// `hash+scope+name` an die Store-Datei an (atomar) und gibt den **Klartext
/// einmalig** zurück. Der Klartext wird nie gespeichert.
pub fn add_key(name: &str, scope: Scope) -> Result<String, KeyMgmtError> {
    if !valid_key_name(name) {
        return Err(KeyMgmtError::InvalidName);
    }
    let _guard = store_mutation_lock();
    let (path, mut entries) = read_store_file()?;
    if entries.iter().any(|e| e.name == name) {
        return Err(KeyMgmtError::Duplicate);
    }
    if entries.len() >= MAX_STORE_KEYS {
        return Err(KeyMgmtError::Full);
    }
    let (plain, hash) = generate_key().map_err(|_| KeyMgmtError::Rng)?;
    entries.push(KeyEntry {
        hash,
        scope,
        name: name.to_string(),
    });
    write_store_atomic(&path, &entries)?;
    Ok(plain)
}

/// Entfernt alle Store-Einträge mit dem Namen *name* und schreibt die Datei
/// atomar neu. Gibt die Zahl der entfernten Einträge zurück (0 = kein Treffer;
/// Env-Bootstrap-Keys stehen nicht in der Datei und sind so nie betroffen).
pub fn remove_keys(name: &str) -> Result<usize, KeyMgmtError> {
    let _guard = store_mutation_lock();
    let (path, mut entries) = read_store_file()?;
    let before = entries.len();
    entries.retain(|e| e.name != name);
    let removed = before - entries.len();
    if removed > 0 {
        write_store_atomic(&path, &entries)?;
    }
    Ok(removed)
}

/// Middleware für `/api/*`: lässt den Request nur mit gültigem Key und
/// ausreichendem Scope durch. Fehlender/ungültiger Key → `401`, zu niedriger
/// Scope → `403`.
pub async fn require_api_key(
    State(auth): State<Arc<RwLock<AuthConfig>>>,
    req: Request,
    next: Next,
) -> Response {
    let required = required_scope(req.method(), req.uri().path());
    // Nur kurz lesend sperren und sofort wieder freigeben; ein vergifteter Lock
    // (Panik in einem anderen Halter) ist beim reinen Lesen unkritisch.
    let decision = {
        let cfg = auth.read().unwrap_or_else(|e| e.into_inner());
        cfg.authorize(req.headers(), required)
    };
    match decision {
        Decision::Allow => next.run(req).await,
        Decision::Unauthorized => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "ungültiger oder fehlender API-Key" })),
        )
            .into_response(),
        Decision::Forbidden => (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "API-Key hat keinen ausreichenden Scope für diese Route" })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Baut eine Config aus Store-Einträgen (hash, scope, name) ohne Env/Datei.
    fn cfg_mit_eintraegen(eintraege: &[(&str, Scope)]) -> AuthConfig {
        let stored = eintraege
            .iter()
            .map(|(klartext, scope)| KeyEntry {
                hash: sha256_hex(klartext),
                scope: *scope,
                name: "test".to_string(),
            })
            .collect();
        AuthConfig::build(&[], stored, false)
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        use axum::http::HeaderName;
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    fn ist_erlaubt(d: Decision) -> bool {
        matches!(d, Decision::Allow)
    }

    fn ist_verboten(d: Decision) -> bool {
        matches!(d, Decision::Forbidden)
    }

    fn ist_unauthorisiert(d: Decision) -> bool {
        matches!(d, Decision::Unauthorized)
    }

    #[test]
    fn offen_ohne_keys() {
        let auth = AuthConfig::build(&[], Vec::new(), false);
        assert!(auth.is_open());
        assert!(ist_erlaubt(auth.authorize(&HeaderMap::new(), Scope::Write)));
    }

    #[test]
    fn fehlender_oder_falscher_key_ist_401() {
        let auth = cfg_mit_eintraegen(&[("geheim", Scope::Read)]);
        assert!(!auth.is_open());
        assert!(ist_unauthorisiert(
            auth.authorize(&HeaderMap::new(), Scope::Read)
        ));
        assert!(ist_unauthorisiert(
            auth.authorize(&headers(&[("x-api-key", "nope")]), Scope::Read)
        ));
    }

    #[test]
    fn sha256_hash_wird_korrekt_abgeglichen() {
        // Bekannter Vektor: sha256("secret").
        assert_eq!(
            sha256_hex("secret"),
            "2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b"
        );
        let auth = cfg_mit_eintraegen(&[("secret", Scope::Read)]);
        assert!(ist_erlaubt(
            auth.authorize(&headers(&[("x-api-key", "secret")]), Scope::Read)
        ));
    }

    #[test]
    fn beide_header_schemata_akzeptiert() {
        let auth = cfg_mit_eintraegen(&[("abc", Scope::Read)]);
        assert!(ist_erlaubt(
            auth.authorize(&headers(&[("x-api-key", "abc")]), Scope::Read)
        ));
        assert!(ist_erlaubt(auth.authorize(
            &headers(&[("authorization", "Bearer abc")]),
            Scope::Read
        )));
        assert!(ist_erlaubt(auth.authorize(
            &headers(&[("authorization", "bearer abc")]),
            Scope::Read
        )));
    }

    #[test]
    fn scope_hierarchie_admin_deckt_read_und_write() {
        assert!(Scope::Admin.covers(Scope::Read));
        assert!(Scope::Admin.covers(Scope::Write));
        assert!(Scope::Admin.covers(Scope::Admin));
        assert!(Scope::Write.covers(Scope::Read));
        assert!(!Scope::Write.covers(Scope::Admin));
        assert!(!Scope::Read.covers(Scope::Write));
    }

    #[test]
    fn read_key_darf_get_aber_nicht_post_download() {
        let auth = cfg_mit_eintraegen(&[("leser", Scope::Read)]);
        let h = headers(&[("x-api-key", "leser")]);
        // GET-artige Route (Read) erlaubt.
        assert!(ist_erlaubt(auth.authorize(&h, Scope::Read)));
        // POST /api/download (Write) verboten → 403.
        assert!(ist_verboten(auth.authorize(&h, Scope::Write)));
    }

    #[test]
    fn write_key_darf_post_download() {
        let auth = cfg_mit_eintraegen(&[("schreiber", Scope::Write)]);
        let h = headers(&[("x-api-key", "schreiber")]);
        assert!(ist_erlaubt(auth.authorize(&h, Scope::Write)));
        assert!(ist_erlaubt(auth.authorize(&h, Scope::Read)));
    }

    #[test]
    fn env_bootstrap_key_ist_admin() {
        // Bootstrap-Keys (Env/with_keys) decken alles ab, auch Write.
        let auth = AuthConfig::build(&["boot".to_string()], Vec::new(), false);
        let h = headers(&[("x-api-key", "boot")]);
        assert!(ist_erlaubt(auth.authorize(&h, Scope::Read)));
        assert!(ist_erlaubt(auth.authorize(&h, Scope::Write)));
        assert!(ist_erlaubt(auth.authorize(&h, Scope::Admin)));
    }

    #[test]
    fn required_scope_mapping() {
        use axum::http::Method;
        assert_eq!(required_scope(&Method::GET, "/api/search"), Scope::Read);
        assert_eq!(required_scope(&Method::GET, "/api/providers"), Scope::Read);
        assert_eq!(required_scope(&Method::GET, "/api/queue"), Scope::Read);
        assert_eq!(required_scope(&Method::POST, "/api/download"), Scope::Write);
    }

    #[test]
    fn store_json_wird_geparst() {
        let json = r#"[
            { "hash": "AABBCC", "scope": "read", "name": "leser" },
            { "hash": "ddeeff", "scope": "write" },
            { "hash": "001122", "scope": "admin", "name": "chef" }
        ]"#;
        let entries = parse_store(json).expect("gültiges JSON");
        assert_eq!(entries.len(), 3);
        // Hash wird lowercase normalisiert.
        assert_eq!(entries[0].hash, "aabbcc");
        assert_eq!(entries[0].scope, Scope::Read);
        assert_eq!(entries[0].name, "leser");
        assert_eq!(entries[1].scope, Scope::Write);
        assert_eq!(entries[1].name, "");
        assert_eq!(entries[2].scope, Scope::Admin);
    }

    #[test]
    fn kaputter_store_json_wird_tolerant_behandelt() {
        // Kein JSON-Array → harter Fehler (sperrt die Auth, kein Fail-open).
        assert!(parse_store("{kaputt").is_err());
        // Array mit einem ungültigen Scope: nur dieser Eintrag wird übersprungen,
        // der gültige bleibt — ein kaputter Datensatz verwirft nicht alle Keys.
        let ok = parse_store(r#"[{"hash":"aa","scope":"superuser"},{"hash":"bb","scope":"read"}]"#)
            .expect("Array bleibt gültig");
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].hash, "bb");
    }

    #[test]
    fn gesperrter_store_ist_nicht_offen() {
        // Store-Ladefehler → deny-all statt offen.
        let auth = AuthConfig::build(&[], Vec::new(), true);
        assert!(!auth.is_open());
        assert!(matches!(
            auth.authorize(&HeaderMap::new(), Scope::Read),
            Decision::Unauthorized
        ));
    }

    #[test]
    fn store_key_mit_scope_greift_ueber_hash() {
        // Realistischer Store-Eintrag: nur der Hash ist hinterlegt, der
        // präsentierte Klartext wird gehasht und abgeglichen.
        let json = format!(
            r#"[{{ "hash": "{}", "scope": "write", "name": "dl" }}]"#,
            sha256_hex("mein-key")
        );
        let entries = parse_store(&json).expect("gültig");
        let auth = AuthConfig::build(&[], entries, false);
        let h = headers(&[("x-api-key", "mein-key")]);
        assert!(ist_erlaubt(auth.authorize(&h, Scope::Write)));
        assert!(ist_unauthorisiert(
            auth.authorize(&headers(&[("x-api-key", "falsch")]), Scope::Read)
        ));
    }

    #[test]
    fn generate_key_hash_passt_zum_klartext() {
        let (klartext, hash) = generate_key().expect("CSPRNG verfügbar");
        assert_eq!(klartext.len(), 64);
        assert_eq!(hash, sha256_hex(&klartext));
        // Zwei Aufrufe liefern unterschiedliche Keys.
        let (k2, _) = generate_key().expect("CSPRNG verfügbar");
        assert_ne!(klartext, k2);
    }

    #[test]
    fn offener_modus_sperrt_admin_routen() {
        let auth = AuthConfig::build(&[], Vec::new(), false); // keine Keys → offen
        assert!(auth.is_open());
        assert!(matches!(
            auth.authorize(&HeaderMap::new(), Scope::Read),
            Decision::Allow
        ));
        // Key-Verwaltung (Admin) bleibt auch offen zu — kein unauth. Key-Anlegen.
        assert!(matches!(
            auth.authorize(&HeaderMap::new(), Scope::Admin),
            Decision::Unauthorized
        ));
    }

    #[test]
    fn key_name_validierung() {
        assert!(valid_key_name("ci-bot_1.key"));
        assert!(!valid_key_name(""));
        assert!(!valid_key_name("hat leerzeichen"));
        assert!(!valid_key_name("slash/raus"));
        assert!(!valid_key_name(&"x".repeat(65)));
    }

    #[test]
    fn ct_eq_matches_only_equal() {
        assert!(ct_eq("same", "same"));
        assert!(!ct_eq("short", "longer"));
        assert!(!ct_eq("aaaa", "aaab"));
    }

    #[test]
    fn serialisierte_einträge_sind_wieder_parsebar() {
        // Store-Roundtrip: schreiben → parsen ergibt dieselben Felder.
        let entries = vec![
            KeyEntry {
                hash: "aabb".to_string(),
                scope: Scope::Read,
                name: "leser".to_string(),
            },
            KeyEntry {
                hash: "ccdd".to_string(),
                scope: Scope::Admin,
                name: "chef".to_string(),
            },
        ];
        let json = serialize_entries(&entries);
        let wieder = parse_store(&json).expect("gültiges Array");
        assert_eq!(wieder.len(), 2);
        assert_eq!(wieder[0].hash, "aabb");
        assert_eq!(wieder[0].scope, Scope::Read);
        assert_eq!(wieder[0].name, "leser");
        assert_eq!(wieder[1].scope, Scope::Admin);
        assert_eq!(wieder[1].name, "chef");
    }

    #[test]
    fn scope_as_str_passt_zum_serde_namen() {
        assert_eq!(Scope::Read.as_str(), "read");
        assert_eq!(Scope::Write.as_str(), "write");
        assert_eq!(Scope::Admin.as_str(), "admin");
    }

    #[test]
    fn required_scope_für_keys_ist_admin() {
        assert_eq!(required_scope(&Method::GET, "/api/keys"), Scope::Admin);
        assert_eq!(required_scope(&Method::POST, "/api/keys"), Scope::Admin);
        assert_eq!(
            required_scope(&Method::DELETE, "/api/keys/ci"),
            Scope::Admin
        );
        // Andere Routen bleiben unverändert.
        assert_eq!(required_scope(&Method::GET, "/api/queue"), Scope::Read);
        assert_eq!(required_scope(&Method::POST, "/api/download"), Scope::Write);
    }
}
