//! Kleiner Ringpuffer für die letzten Log-Zeilen (Diagnose-Panel). Die fmt-Ausgabe
//! wird zusätzlich nach stderr geteet, damit das Terminal weiter alles sieht.
//!
//! Es landet nur, was ohnehin geloggt wird — die Logging-Disziplin im Kern hält
//! Secrets/Token draußen (es werden z. B. nur Hosts, keine signierten URLs geloggt).
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
                b.push_back(line.to_string());
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
