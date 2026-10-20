mod certs;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::http::header;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use clap::Parser;
use mime_guess::from_path;
use rust_embed::Embed;
use tokio::sync::Notify;

/// Embedded static assets from web/ and the WASM pkg.
#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web"]
#[prefix = "web/"]
struct WebAsset;

#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../mixer-wasm/pkg"]
#[prefix = "pkg/"]
struct WasmAsset;

/// How the web server's TLS cert is obtained.
enum CertMode {
    /// Self-signed: persisted 13-month PEM (certs.rs).
    SelfSigned,
    /// User-provided PEM files (--cert-pem/--key-pem, e.g. mkcert or a real
    /// CA); the browser validates them via its trust store.
    Pem { cert: PathBuf, key: PathBuf },
}

#[derive(Parser)]
#[command(name = "cakemix", about = "CakeMix — WASM Audio Mixer")]
struct Cli {
    #[arg(short, long, default_value = "8200")]
    port: u16,

    #[arg(
        long,
        default_value = "0.0.0.0",
        help = "Bind address for the web server"
    )]
    bind: String,

    #[arg(long, help = "Serve the web UI over plain HTTP")]
    no_tls: bool,

    #[arg(
        long,
        requires = "key_pem",
        help = "PEM cert path (mkcert/real CA) for web TLS"
    )]
    cert_pem: Option<PathBuf>,

    #[arg(
        long,
        requires = "cert_pem",
        help = "PEM key path (mkcert/real CA) for web TLS"
    )]
    key_pem: Option<PathBuf>,
}

async fn serve_index() -> Response {
    match WebAsset::get("web/index.html") {
        Some(content) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
            ],
            content.data.to_vec(),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve_web_file(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Try web/ assets first (index.html, app.js, styles.css, etc.)
    let web_path = if path.starts_with("web/") {
        path.to_string()
    } else if path.is_empty() {
        "web/index.html".to_string()
    } else {
        format!("web/{path}")
    };

    if let Some(content) = WebAsset::get(&web_path) {
        let mime = from_path(&web_path).first_or_octet_stream();
        return (
            [
                (header::CONTENT_TYPE, mime.as_ref()),
                (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
            ],
            content.data.to_vec(),
        )
            .into_response();
    }

    // Try pkg/ assets (wasm JS glue + .wasm binary)
    let pkg_path = if path.starts_with("pkg/") {
        path.to_string()
    } else if path.contains("mixer_wasm") {
        format!("pkg/{path}")
    } else {
        path.to_string()
    };

    if let Some(content) = WasmAsset::get(&pkg_path) {
        let mime = from_path(&pkg_path).first_or_octet_stream();
        // WASM needs correct MIME type.
        let mime_str = if pkg_path.ends_with(".wasm") {
            "application/wasm"
        } else {
            mime.as_ref()
        };
        return (
            [
                (header::CONTENT_TYPE, mime_str),
                (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate"),
            ],
            content.data.to_vec(),
        )
            .into_response();
    }

    StatusCode::NOT_FOUND.into_response()
}

async fn health() -> &'static str {
    "ok"
}

fn build_router() -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/health", get(health))
        .route("/api/cert-hash", get(api_cert_hash))
        .fallback(serve_web_file)
        .layer(axum::middleware::from_fn(coop_coep_headers))
}

/// Every response — routes and the fallback static handler alike, in both TLS
/// and --no-tls modes — carries the COOP/COEP pair: the QuicSRT receive and
/// publish engine (emscripten + pthreads → SharedArrayBuffer) requires a
/// cross-origin-isolated page. Limitation: over plain HTTP (--no-tls) the
/// browser only grants `crossOriginIsolated` on localhost (secure context);
/// remote LAN clients need the TLS mode.
async fn coop_coep_headers(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let mut res = next.run(req).await;
    res.headers_mut().insert(
        header::HeaderName::from_static("cross-origin-opener-policy"),
        header::HeaderValue::from_static("same-origin"),
    );
    res.headers_mut().insert(
        header::HeaderName::from_static("cross-origin-embedder-policy"),
        header::HeaderValue::from_static("require-corp"),
    );
    res
}

/// Proxy a remote gateway's `cert-hash.js` (the browser cannot fetch another
/// origin's cert-hash.js — CORS; pattern copied from SlopShady's
/// /api/stream/cert-hash). The caller passes the gateway's WEB URL — the
/// page the user browses to; cert-hash.js is served same-origin there. One
/// fetch yields both CERT_HASH (hex for self-signed pinning, null for
/// PKI/mkcert) and WT_PORT, which is everything the browser needs to build
/// the WT URL and pin the cert.
///
/// TLS policy: self-signed gateways are a closed-network convenience. Only
/// loopback/private/link-local hosts (or dotless/`.local`-style names) are
/// fetched with `danger_accept_invalid_certs(true)`; public hosts must present
/// a certificate a normal client already trusts — no unsafe allowance online.
/// When the fetched `cert-hash.js` yields a non-null hash, the real trust
/// anchor is still the client-side WebTransport `serverCertificateHashes`
/// pinning built from that hash.
#[derive(serde::Deserialize)]
struct CertHashParams {
    url: String,
}

/// Whether a gateway host is on a closed network: loopback/private/link-local
/// IP literal, `localhost`, a dotless hostname, or a `.local`/`.lan`/
/// `.internal` name. Only these may be fetched with self-signed cert
/// validation; public hosts require a PKI-valid certificate.
fn closed_network_host(host: &str) -> bool {
    let h = host
        .trim_end_matches('.')
        .trim_start_matches('[')
        .trim_end_matches(']');
    if h.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = h.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
            std::net::IpAddr::V6(v6) => {
                let s = v6.segments();
                v6.is_loopback()
                    || (s[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                    || (s[0] & 0xffc0) == 0xfe80 // link local fe80::/10
            }
        };
    }
    !h.contains('.') || h.ends_with(".local") || h.ends_with(".lan") || h.ends_with(".internal")
}

async fn api_cert_hash(
    axum::extract::Query(params): axum::extract::Query<CertHashParams>,
) -> Response {
    let (cert_url, host) = match validate_gateway_target(&params.url) {
        Ok(u) => u,
        Err(e) => return cert_hash_json(None, None, Some(&e)),
    };
    let client = match reqwest::Client::builder()
        .danger_accept_invalid_certs(closed_network_host(&host))
        .timeout(std::time::Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(e) => return cert_hash_json(None, None, Some(&format!("client build: {e}"))),
    };
    match client.get(&cert_url).send().await {
        Ok(r) if r.status().is_success() => {
            let text = r.text().await.unwrap_or_default();
            cert_hash_json(extract_cert_hash(&text), extract_wt_port(&text), None)
        }
        Ok(r) => cert_hash_json(
            None,
            None,
            Some(&format!("upstream status {} for {}", r.status(), cert_url)),
        ),
        Err(e) => cert_hash_json(
            None,
            None,
            Some(&format!("fetch failed for {cert_url}: {e}")),
        ),
    }
}

/// Validate a caller-supplied gateway URL before this server fetches its
/// cert-hash.js. Returns the cert-hash.js URL to fetch plus the host (for
/// the closed-network TLS policy in `closed_network_host`).
///
/// The endpoint exists because a browser cannot fetch another origin's
/// cert-hash.js (CORS) — and for this product the legitimate targets are
/// OTHER MACHINES ON THE LAN (the multi-session receive architecture:
/// one QuicSRT session per remote source), so private/loopback/link-local
/// addresses are deliberately ALLOWED. Validation is https-only (WebTransport
/// requires TLS) plus a parseable URL with a host.
///
/// The trust anchor is not TLS here — it is the client-side
/// `serverCertificateHashes` pinning done from the hash this proxy returns
/// when non-null. For public hosts the fetch itself is strictly validated
/// (PKI); the self-signed allowance applies only to closed-network hosts.
fn validate_gateway_target(raw: &str) -> Result<(String, String), String> {
    let parsed = url::Url::parse(raw).map_err(|_| "invalid gateway url".to_string())?;
    if parsed.scheme() != "https" {
        return Err(format!(
            "gateway url must be https, got {}",
            parsed.scheme()
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "gateway url has no host".to_string())?
        .to_string();
    let port = parsed.port_or_known_default().unwrap_or(443);

    // url serializes IPv6 hosts WITH brackets ("[::1]"), which is exactly
    // what the fetch URL needs — no stripping required.
    Ok((format!("https://{host}:{port}/cert-hash.js"), host))
}

fn cert_hash_json(hash: Option<String>, wt_port: Option<u16>, error: Option<&str>) -> Response {
    let body = serde_json::json!({
        "hash": hash,
        "wtPort": wt_port,
        "error": error,
    });
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body.to_string(),
    )
        .into_response()
}

/// Parse `CERT_HASH = "…" | null` from the cert-hash.js body. Returns the hex
/// hash string, or None for null/missing.
fn extract_cert_hash(text: &str) -> Option<String> {
    let idx = text.find("CERT_HASH")?;
    let after = text[idx + "CERT_HASH".len()..].trim_start();
    let after = after.strip_prefix('=')?.trim_start();
    if let Some(rest) = after.strip_prefix('"') {
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    } else {
        None
    }
}

/// Parse `WT_PORT = <number>;` from the cert-hash.js body. None when absent
/// (older gateways that don't advertise it — caller falls back to 4433).
fn extract_wt_port(text: &str) -> Option<u16> {
    let idx = text.find("WT_PORT")?;
    let after = text[idx + "WT_PORT".len()..].trim_start();
    let after = after.strip_prefix('=')?.trim_start();
    let end = after.find(|c: char| !c.is_ascii_digit())?;
    after[..end].parse().ok()
}

fn read_pem_file(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("Error: could not read {}: {e}", path.display());
        std::process::exit(1);
    })
}

/// Bind the web port up front (fail fast), then serve TLS or plain HTTP as a
/// background task that exits when `shutdown` fires.
async fn spawn_web_server(
    addr: SocketAddr,
    app: Router,
    web_tls: Option<(Vec<u8>, Vec<u8>)>,
    shutdown: Arc<Notify>,
) -> anyhow::Result<tokio::task::JoinHandle<anyhow::Result<()>>> {
    match web_tls {
        Some((cert_pem, key_pem)) => {
            // axum-server's tls-rustls default provider is aws-lc-rs while
            // reqwest's rustls-tls links ring; with both providers in the
            // process, rustls cannot pick a process default by itself and the
            // TLS acceptor panics on first use. Install ring explicitly (the
            // same provider the old websrt-based binary installed for
            // quinn/wtransport) before the acceptor is built.
            let _ = rustls::crypto::ring::default_provider().install_default();

            let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem(cert_pem, key_pem)
                .await
                .map_err(|e| anyhow::anyhow!("failed to load TLS certificate: {e}"))?;

            let listener = std::net::TcpListener::bind(addr)
                .map_err(|e| anyhow::anyhow!("could not bind to {addr}: {e}"))?;
            let handle = axum_server::Handle::new();
            let shutdown_handle = handle.clone();
            tokio::spawn(async move {
                shutdown.notified().await;
                shutdown_handle.shutdown();
            });

            Ok(tokio::spawn(async move {
                axum_server::tls_rustls::from_tcp_rustls(listener, tls_config)
                    .handle(handle)
                    .serve(app.into_make_service())
                    .await
                    .map_err(|e| anyhow::anyhow!("web server: {e}"))
            }))
        }
        None => {
            let listener = tokio::net::TcpListener::bind(addr).await?;
            Ok(tokio::spawn(async move {
                let graceful = async move {
                    shutdown.notified().await;
                };
                axum::serve(listener, app)
                    .with_graceful_shutdown(graceful)
                    .await
                    .map_err(|e| anyhow::anyhow!("web server: {e}"))
            }))
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let mode = match (cli.cert_pem.clone(), cli.key_pem.clone()) {
        (Some(cert), Some(key)) => CertMode::Pem { cert, key },
        (None, None) => CertMode::SelfSigned,
        _ => unreachable!("clap `requires` enforces the PEM flag pair"),
    };

    // Web TLS material (None in --no-tls dev mode).
    let web_tls = if cli.no_tls {
        None
    } else {
        match &mode {
            CertMode::SelfSigned => {
                let cert = certs::ensure_web_cert(Path::new("cert.pem"), Path::new("key.pem"))
                    .unwrap_or_else(|e| {
                        eprintln!("Error: failed to prepare web certificate: {e}");
                        std::process::exit(1);
                    });
                Some((cert.cert_pem, cert.key_pem))
            }
            CertMode::Pem { cert, key } => {
                println!("web TLS: using {} + {}", cert.display(), key.display());
                Some((read_pem_file(cert), read_pem_file(key)))
            }
        }
    };

    let bind_addr: std::net::IpAddr = cli
        .bind
        .parse()
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    let addr = SocketAddr::new(bind_addr, cli.port);

    let scheme = if cli.no_tls { "http" } else { "https" };
    let url = format!("{scheme}://localhost:{}", cli.port);

    println!("CakeMix server starting");
    println!("  web:        {url} (bind {})", cli.bind);
    println!(
        "  gateway:    external QuicSRT-gateway (browser connects via the drawer URL; default https://127.0.0.1:8443)"
    );
    match &mode {
        CertMode::SelfSigned => {
            println!("  cert mode:  self-signed (web PEM persisted 13 months)")
        }
        CertMode::Pem { cert, .. } => println!("  cert mode:  pem {}", cert.display()),
    }
    if cli.no_tls {
        println!("  web TLS:    disabled (--no-tls)");
    }
    println!("Press Ctrl+C to stop.");

    let shutdown = Arc::new(Notify::new());
    tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            let _ = tokio::signal::ctrl_c().await;
            shutdown.notify_waiters();
        }
    });

    // Open browser after a short delay.
    let open_url = url.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let _ = open::that(&open_url);
    });

    let axum_task = spawn_web_server(addr, build_router(), web_tls, shutdown.clone())
        .await
        .unwrap_or_else(|e| {
            eprintln!("Error: {e}");
            std::process::exit(1);
        });

    // Exit on ctrl-c, or as soon as the web server task fails.
    tokio::pin!(axum_task);

    enum Exited {
        CtrlC,
        WebServer,
    }
    let exited = tokio::select! {
        _ = shutdown.notified() => Exited::CtrlC,
        res = &mut axum_task => {
            if let Ok(Err(e)) = res {
                eprintln!("Error: web server failed: {e}");
            }
            Exited::WebServer
        }
    };
    if let Exited::CtrlC = exited {
        println!("Shutting down.");
    }
    shutdown.notify_waiters();

    // Drain: the axum task exits via its shutdown watcher/handle. Tasks that
    // already finished (the select! winner) are not awaited again.
    if !matches!(exited, Exited::WebServer) {
        let _ = (&mut axum_task).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{build_router, spawn_web_server, validate_gateway_target, SocketAddr};

    #[test]
    fn accepts_public_ip_literal() {
        let (url, host) = validate_gateway_target("https://203.0.113.7:8201/").unwrap();
        assert_eq!(url, "https://203.0.113.7:8201/cert-hash.js");
        assert_eq!(host, "203.0.113.7");
    }

    #[test]
    fn accepts_public_dns_name_and_preserves_port_and_path() {
        // Port comes from the URL (no explicit port → https default 443);
        // query strings are dropped — only the cert-hash.js path matters.
        let (url, host) =
            validate_gateway_target("https://mixer.example.com:5173/?stream=x").unwrap();
        assert_eq!(url, "https://mixer.example.com:5173/cert-hash.js");
        assert_eq!(host, "mixer.example.com");
    }

    #[test]
    fn rejects_non_https_scheme() {
        let err = validate_gateway_target("http://gateway.example.com/").unwrap_err();
        assert!(err.contains("must be https"), "got: {err}");
    }

    #[test]
    fn rejects_invalid_urls() {
        for raw in ["", "not a url", "https://"] {
            assert_eq!(
                validate_gateway_target(raw).unwrap_err(),
                "invalid gateway url"
            );
        }
    }

    #[test]
    fn accepts_private_and_local_ip_literals() {
        // CakeMix consumes gateways on OTHER LAN MACHINES — private ranges
        // are the primary workflow, not an exception.
        for raw in [
            "https://127.0.0.1/",
            "https://[::1]/",
            "https://10.0.0.5/",
            "https://10.26.200.105:8201/",
            "https://192.168.1.214:5173/",
            "https://172.16.0.9/",
            "https://169.254.1.1/",
            "https://100.64.0.1/",    // CGNAT
            "https://100.127.255.9/", // CGNAT top edge
            "https://0.0.0.0/",
            "https://[fd00::5]/", // IPv6 unique-local
            "https://[::]/",      // unspecified v6
        ] {
            let (url, _) = validate_gateway_target(raw).unwrap_or_else(|e| panic!("{raw}: {e}"));
            assert!(url.ends_with("/cert-hash.js"), "{raw}: got {url}");
        }
    }

    #[test]
    fn accepts_internal_hostnames() {
        // Same policy as the IP literals above: a second gateway on this
        // machine (localhost) is a legitimate target.
        for raw in [
            "https://localhost:4433/",
            "https://LOCALHOST/", // case-insensitive
            "https://gateway.local/",
            "https://foo.internal/",
        ] {
            let (url, _) = validate_gateway_target(raw).unwrap_or_else(|e| panic!("{raw}: {e}"));
            assert!(url.ends_with("/cert-hash.js"), "{raw}: got {url}");
        }
    }

    #[test]
    fn allows_named_hosts_that_merely_look_like_ips() {
        // A NAME that embeds a private IP is not that IP — it fetches like
        // any other name.
        assert!(validate_gateway_target("https://10.0.0.5.nip.io/").is_ok());
    }

    #[tokio::test]
    async fn tls_mode_serves_and_sends_coop_coep() {
        // Regression (caught by the supervised instance, which runs TLS mode
        // while CI/dev use --no-tls): axum-server (aws-lc-rs) and reqwest
        // (ring) both link rustls providers, so rustls cannot auto-pick a
        // process default and the TLS acceptor panicked on first use.
        // spawn_web_server must install a provider itself — this test
        // process installs nothing, so a missing install fails the fetch.
        use axum::http::header::HeaderMap;
        use std::sync::Arc;
        use std::time::Duration;
        use tokio::sync::Notify;
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "localhost");
        let key_pair = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key_pair).unwrap();
        let cert_pem = cert.pem().into_bytes();
        let key_pem = key_pair.serialize_pem().into_bytes();

        // Ephemeral port: bind, read, drop — spawn_web_server rebinds it.
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let shutdown = Arc::new(Notify::new());
        let task = spawn_web_server(
            SocketAddr::from(([127, 0, 0, 1], port)),
            build_router(),
            Some((cert_pem, key_pem)),
            shutdown.clone(),
        )
        .await
        .unwrap();

        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let mut last_err = String::new();
        let mut served = false;
        for _ in 0..50 {
            match client
                .get(format!("https://127.0.0.1:{port}/health"))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => {
                    let headers: HeaderMap = r.headers().clone();
                    assert_eq!(
                        headers.get("cross-origin-opener-policy").unwrap(),
                        "same-origin",
                        "COOP header missing in TLS mode"
                    );
                    assert_eq!(
                        headers.get("cross-origin-embedder-policy").unwrap(),
                        "require-corp",
                        "COEP header missing in TLS mode"
                    );
                    assert_eq!(r.text().await.unwrap(), "ok");
                    served = true;
                    break;
                }
                Ok(r) => last_err = format!("status {}", r.status()),
                Err(e) => last_err = e.to_string(),
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        shutdown.notify_waiters();
        let _ = task.await;
        assert!(served, "TLS web server never became reachable: {last_err}");
    }
}
