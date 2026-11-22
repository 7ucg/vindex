//! Korrigiert von Hand getippte Portal-URLs.
//!
//! Angelehnt an `url_autocorrect.py` der Referenz, aber auf die drei von vindex
//! unterstützten Portale beschränkt und nur mit den deterministischen Fixes:
//! Copy-Paste-Müll abschneiden, Schema auf `https` zwingen, Host-Tippfehler
//! korrigieren. Keine Slug-Fuzzy-Korrektur (die bräuchte einen Titel-Cache).

/// Die Hosts, auf die korrigiert wird (Kanonische Form).
const CANONICAL_HOSTS: &[&str] = &["aniworld.to", "s.to", "serienstream.to", "filmo.to"];

/// Zeichen, in die ein Copy-Paste eine URL gern einwickelt.
const WRAPPING: &[char] = &[
    '<', '>', '"', '\'', '`', '«', '»', '(', ')', '[', ']', '{', '}', ' ', '\t', '\r', '\n',
];
/// Satzzeichen, die hinten hängen bleiben.
const TRAILING_JUNK: &[char] = &['.', ',', ';', ':', '!', '?'];

/// Eine korrigierte URL plus kurze Notiz, was geändert wurde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correction {
    pub url: String,
    pub reason: String,
}

/// Versucht, *input* zu einer brauchbaren Portal-URL zu reparieren. `None`, wenn
/// kein unterstützter Host erkennbar ist (dann soll der Aufrufer den Fehler zeigen).
pub fn autocorrect(input: &str) -> Option<Correction> {
    let mut notes: Vec<&str> = Vec::new();

    let trimmed = strip_wrapping(input);
    if trimmed != input.trim() {
        notes.push("Wrapping/Satzzeichen entfernt");
    }

    // Schema abschneiden (wird ohnehin durch https ersetzt) und führende Slashes weg.
    let body = strip_scheme(&trimmed);
    let body = body.trim_start_matches('/');
    if body.is_empty() {
        return None;
    }

    // In Host und Rest teilen.
    let (raw_host, rest) = match body.split_once('/') {
        Some((h, r)) => (h, format!("/{r}")),
        None => (body, String::new()),
    };
    let raw_host = raw_host.split('?').next().unwrap_or(raw_host);
    let host_lc = raw_host.to_ascii_lowercase();
    let host_lc = host_lc.strip_prefix("www.").unwrap_or(&host_lc);

    let host = canonical_host(host_lc)?;
    if host != host_lc {
        notes.push("Host korrigiert");
    }

    // Pfad säubern (trailing Slash weg, aber Query/Fragment behalten).
    let path = rest.trim_end_matches('/');
    let url = format!("https://{host}{path}");

    let reason = if notes.is_empty() {
        "unverändert".to_string()
    } else {
        notes.join(", ")
    };
    Some(Correction { url, reason })
}

/// Entfernt Wrapping-Zeichen vorne und Wrapping/Satzzeichen hinten.
fn strip_wrapping(raw: &str) -> String {
    let mut text = raw.trim();
    while let Some(first) = text.chars().next() {
        if WRAPPING.contains(&first) {
            text = &text[first.len_utf8()..];
        } else {
            break;
        }
    }
    while let Some(last) = text.chars().last() {
        if WRAPPING.contains(&last) || TRAILING_JUNK.contains(&last) {
            text = &text[..text.len() - last.len_utf8()];
        } else {
            break;
        }
    }
    text.to_string()
}

/// Schneidet ein führendes `schema://` (oder `schema:/`) ab.
fn strip_scheme(text: &str) -> String {
    if let Some(pos) = text.find(":/") {
        let scheme = &text[..pos];
        if !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        {
            let after = &text[pos + 1..];
            return after.trim_start_matches('/').to_string();
        }
    }
    text.to_string()
}

/// Bildet einen (ggf. vertippten) Host auf einen kanonischen Portal-Host ab.
fn canonical_host(host: &str) -> Option<&'static str> {
    // Exakter Treffer.
    if let Some(h) = CANONICAL_HOSTS.iter().find(|h| **h == host) {
        return Some(h);
    }
    // Gleicher Namensstamm, andere TLD (aniworld.tv → aniworld.to).
    let stem = host.split('.').next().unwrap_or(host);
    let by_stem: Option<&'static str> = match stem {
        "aniworld" => Some("aniworld.to"),
        "serienstream" => Some("serienstream.to"),
        "filmo" => Some("filmo.to"),
        _ => None,
    };
    if by_stem.is_some() {
        return by_stem;
    }
    // Leichter Tippfehler (Editierdistanz ≤ 2) zu genau einem Host.
    let mut hit: Option<&'static str> = None;
    for cand in CANONICAL_HOSTS {
        if levenshtein(host, cand) <= 2 {
            if hit.is_some() {
                return None; // mehrdeutig → lieber nicht raten
            }
            hit = Some(cand);
        }
    }
    hit
}

/// Editierdistanz (Levenshtein), für die Host-Tippfehler-Erkennung.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(input: &str) -> Option<String> {
        autocorrect(input).map(|c| c.url)
    }

    #[test]
    fn forces_https_and_trims() {
        assert_eq!(
            fixed("  http://aniworld.to/anime/stream/x/staffel-1/episode-1  "),
            Some("https://aniworld.to/anime/stream/x/staffel-1/episode-1".into())
        );
        assert_eq!(
            fixed("\"https://filmo.to/movies/y\""),
            Some("https://filmo.to/movies/y".into())
        );
    }

    #[test]
    fn fixes_host_typos() {
        assert_eq!(
            fixed("aniworld.tv/anime/stream/x"),
            Some("https://aniworld.to/anime/stream/x".into())
        );
        assert_eq!(
            fixed("https://www.serienstream.cc/serie/stream/z"),
            Some("https://serienstream.to/serie/stream/z".into())
        );
        // Editierdistanz-Tippfehler.
        assert_eq!(fixed("aniworld.ti/x"), Some("https://aniworld.to/x".into()));
    }

    #[test]
    fn rejects_unknown_host() {
        assert_eq!(fixed("https://evil.example/aniworld.to"), None);
        assert_eq!(fixed("einfach text"), None);
    }

    #[test]
    fn levenshtein_basic() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("abc", "abc"), 0);
    }
}
