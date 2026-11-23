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

/// Reduziert eine http(s)-URL auf `scheme://host/…` (bzw. `scheme://host`).
/// Pfad und Query — und damit alle signierten Token — fallen weg.
fn redact_url(url: &str) -> String {
    let scheme = if url.starts_with("https") {
        "https"
    } else {
        "http"
    };
    let after = url.splitn(2, "://").nth(1).unwrap_or("");
    let host = after.split(['/', '?', '#']).next().unwrap_or("");
    if after.len() > host.len() {
        format!("{scheme}://{host}/…")
    } else {
        format!("{scheme}://{host}")
    }
}

/// Ersetzt in einer Zeile jede http(s)-URL durch ihre redigierte Form.
fn scrub(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    loop {
        // Früheste der beiden URL-Marken wählen (sonst würde eine frühere https-URL
        // vor einer späteren http-URL unredigiert durchrutschen).
        let pos = match (rest.find("http://"), rest.find("https://")) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => break,
        };
        out.push_str(&rest[..pos]);
        let url_rest = &rest[pos..];
        // URL endet an Whitespace oder einem typischen Begrenzer.
        let end = url_rest
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`' | ')' | ']' | ',')
            })
            .unwrap_or(url_rest.len());
        out.push_str(&redact_url(&url_rest[..end]));
        rest = &url_rest[end..];
    }
    out.push_str(rest);
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
}
