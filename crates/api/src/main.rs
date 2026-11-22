//! Standalone-Daemon-Einstieg für die vindex API.
//!
//! Adresse: `--addr <host:port>` (erstes Argument) hat Vorrang, sonst die
//! Umgebungsvariable `VINDEX_API_ADDR`, sonst `127.0.0.1:8080`.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = addr_from_args(std::env::args().skip(1))
        .or_else(|| std::env::var("VINDEX_API_ADDR").ok())
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());
    vindex_api::serve(&addr).await
}

/// Liest `--addr <wert>` oder `--addr=<wert>` aus den Argumenten.
fn addr_from_args(args: impl Iterator<Item = String>) -> Option<String> {
    let mut args = args;
    while let Some(arg) = args.next() {
        if let Some(v) = arg.strip_prefix("--addr=") {
            return Some(v.to_string());
        }
        if arg == "--addr" {
            return args.next();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::addr_from_args;

    fn parse(args: &[&str]) -> Option<String> {
        addr_from_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn addr_flag_forms() {
        assert_eq!(
            parse(&["--addr", "0.0.0.0:9000"]).as_deref(),
            Some("0.0.0.0:9000")
        );
        assert_eq!(
            parse(&["--addr=127.0.0.1:8081"]).as_deref(),
            Some("127.0.0.1:8081")
        );
        assert_eq!(parse(&["serve", "--addr", "x:1"]).as_deref(), Some("x:1"));
        assert_eq!(parse(&["--other", "y"]), None);
        assert_eq!(parse(&[]), None);
        assert_eq!(parse(&["--addr"]), None); // Wert fehlt
    }
}
