//! Kleiner Ringpuffer für die letzten Log-Zeilen (Diagnose-Panel). Die fmt-Ausgabe
//! wird zusätzlich nach stderr geteet, damit das Terminal weiter alles sieht.
//!
//! Der Puffer ist über einen Tauri-Command (Kopieren/Export) nach außen sichtbar,
//! deshalb wird **jede Zeile vor dem Puffern redigiert**: aus jeder http(s)-URL
//! bleibt nur `scheme://host/…`, Pfad und Query (und damit signierte Token, `t=`,
//! `token=` usw.) fallen weg. Das greift auch, wenn ein Fehler-`Display` (z. B. von
//! reqwest) eine volle URL mitführt — man verlässt sich nicht auf die Call-Site.
//! stderr (nur Dev-Terminal, nicht exportierbar) sieht weiterhin die Rohzeile.
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

const MAX_LINES: usize = 500;

pub type LogBuf = Arc<Mutex<VecDeque<String>>>;

pub fn new_buffer() -> LogBuf {
    Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LINES)))
}

pub fn snapshot(buf: &LogBuf) -> Vec<String> {
    buf.lock().unwrap().iter().cloned().collect()
}

/// True, wenn das Zeichen eine URL beendet (Whitespace oder typische Begrenzer).
fn is_url_end(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`' | ')' | ']' | ',')
}

/// Redigiert in einer Zeile jedes `://`-Vorkommen: Text bis **einschließlich** `://`
/// bleibt unverändert, aber alles danach (Authority bis zum nächsten Begrenzer) wird
/// auf `host` reduziert — userinfo (`user:pass@`) verworfen, Pfad und Query (und damit
/// signierte Token) fallen weg. Bewusst ohne Schema-Prüfung: so kann kein Token über
/// ein „ungültiges" Schema (z. B. an Text klebendes `2https://…`) durchrutschen.
fn scrub(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut copied = 0usize; // bis hierhin ist das Original schon in `out`
    let mut i = 0usize;
    while let Some(rel) = line[i..].find("://") {
        let after = i + rel + 3; // hinter "://"
        let end_rel = line[after..].find(is_url_end).unwrap_or(line.len() - after);
        let url_end = after + end_rel;

        let authority_and_rest = &line[after..url_end];
        let authority = authority_and_rest
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("");
        // userinfo verwerfen: nur das, was hinter dem letzten '@' steht (host[:port]).
        let host = authority.rsplit('@').next().unwrap_or(authority);

        // Alles bis einschließlich "://" unverändert übernehmen, dann nur den Host.
        out.push_str(&line[copied..after]);
        out.push_str(host);
        if authority_and_rest.len() > authority.len() {
            out.push_str("/…");
        }
        copied = url_end;
        i = url_end;
    }
    out.push_str(&line[copied..]);
    out
}

/// `MakeWriter` für `tracing_subscriber::fmt`: schreibt pro Event in den Ringpuffer
/// und nach stderr.
#[derive(Clone)]
pub struct BufferMaker {
    buf: LogBuf,
}

impl BufferMaker {
    pub fn new(buf: LogBuf) -> Self {
        Self { buf }
    }
}

pub struct LineWriter {
    buf: LogBuf,
    bytes: Vec<u8>,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferMaker {
    type Writer = LineWriter;
    fn make_writer(&'a self) -> Self::Writer {
        LineWriter {
            buf: self.buf.clone(),
            bytes: Vec::new(),
        }
    }
}

impl Write for LineWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let _ = io::stderr().write_all(data);
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.bytes.is_empty() {
            return Ok(());
        }
        let chunk = std::mem::take(&mut self.bytes);
        if let Ok(s) = String::from_utf8(chunk) {
            let mut b = self.buf.lock().unwrap();
            for line in s.lines() {
                let line = line.trim_end();
                if line.is_empty() {
                    continue;
                }
                if b.len() >= MAX_LINES {
                    b.pop_front();
                }
                b.push_back(scrub(line));
            }
        }
        Ok(())
    }
}

impl Drop for LineWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::scrub;

    #[test]
    fn strips_path_and_query() {
        assert_eq!(
            scrub("Stream https://cdn.example.com/hls/SECRET/master.m3u8?token=abc123&t=99 ok"),
            "Stream https://cdn.example.com/… ok"
        );
    }

    #[test]
    fn handles_two_urls_earliest_first() {
        let got = scrub("a https://a.tld/x?tok=1 b http://b.tld/y?t=2 c");
        assert_eq!(got, "a https://a.tld/… b http://b.tld/… c");
    }

    #[test]
    fn host_only_url_unchanged_shape() {
        assert_eq!(
            scrub("ping https://host.tld done"),
            "ping https://host.tld done"
        );
    }

    #[test]
    fn no_url_is_identity() {
        assert_eq!(scrub("just a plain line"), "just a plain line");
    }

    #[test]
    fn drops_userinfo_credentials() {
        assert_eq!(
            scrub("auth https://user:s3cr3t@host.tld:8443/path?x=1 end"),
            "auth https://host.tld:8443/… end"
        );
    }

    #[test]
    fn case_insensitive_and_generic_scheme() {
        assert_eq!(
            scrub("x HTTPS://H.TLD/p?tok=1 y rtmp://media.tld/live/KEY z"),
            "x HTTPS://H.TLD/… y rtmp://media.tld/… z"
        );
    }

    #[test]
    fn bare_colon_slashes_without_host_path_untouched() {
        // "://4" hat keinen Pfad → bleibt wie es ist (nichts Sensibles).
        assert_eq!(scrub("ratio 3 ://4 done"), "ratio 3 ://4 done");
    }

    #[test]
    fn digit_glued_scheme_still_redacted() {
        // Parser-Differential: ein an Text klebendes „2https://…" darf den Token
        // nicht über ein vermeintlich ungültiges Schema durchlassen.
        let got = scrub("episode 2https://cdn.tld/x?token=secret done");
        assert!(!got.contains("secret"), "Token durchgerutscht: {got}");
        assert_eq!(got, "episode 2https://cdn.tld/… done");
    }
}
