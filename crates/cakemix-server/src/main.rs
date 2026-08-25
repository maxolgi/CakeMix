mod certs;
mod gateway;
mod ref_web;

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
use websrt::cert::{Cert, CertSource};

/// Embedded static assets from web/ and the WASM pkg.
#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web"]
#[prefix = "web/"]
struct WebAsset;

#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../mixer-wasm/pkg"]
#[prefix = "pkg/"]
struct WasmAsset;

/// How the two TLS surfaces (web HTTPS + WebTransport identity) get their cert.
enum CertMode {
    /// Self-signed: persisted 13-month PEM for the web server (certs.rs) plus a
    /// fresh ≤14-day WT identity whose DER SHA-256 is pinned by the browser.
    SelfSigned,
    /// User-provided PEM files (--cert-pem/--key-pem, e.g. mkcert or a real
    /// CA) used for BOTH the web TLS and the WT identity; the browser
    /// validates via its trust store, so no cert hash is advertised.
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

    #[arg(
        long,
        help = "Serve the web UI over plain HTTP (WebTransport + SRT still run)"
    )]
    no_tls: bool,

    #[arg(long, default_value_t = 4433u16, help = "WebTransport UDP port")]
    wt_port: u16,

    // Legacy direct-ingest leg (OBS/fixture → this binary). Kept for
    // compatibility; NOT the test path — sources publish to the user's
    // websrt-gateway and CakeMix subscribes over WebSRT. See AGENTS.md.
    #[arg(long, default_value_t = 9001u16, help = "SRT ingest UDP port")]
    srt_port: u16,

    // Reference web UI (vendor/WebSRT/web demo app: viewer, publisher, debug
    // panels) on its own HTTPS port, mirroring websrt-gateway's --web-port.
    #[arg(
        long,
        default_value_t = 8201u16,
        help = "HTTPS port for the reference WebSRT web UI (0 disables)"
    )]
    web_port: u16,

    #[arg(
        long,
        default_value = "0.0.0.0",
        help = "Bind address for the reference web UI"
    )]
    web_bind: String,

    #[arg(long, default_value_t = 1000u64, value_parser = clap::value_parser!(u64).range(1..),
          help = "SRT TSBPD latency in ms (gateway and ingest legs)")]
    latency_ms: u64,

    #[arg(
        long,
        requires = "key_pem",
        help = "PEM cert path (mkcert/real CA) for web TLS + WebTransport"
    )]
    cert_pem: Option<PathBuf>,

    #[arg(
        long,
        requires = "cert_pem",
        help = "PEM key path (mkcert/real CA) for web TLS + WebTransport"
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

fn build_router(
    cert_hash_js: String,
    stats: std::sync::Arc<websrt::gateway::GatewayStatsHandle>,
) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/health", get(health))
        .route("/api/cert-hash", get(api_cert_hash))
        .route(
            "/api/streams",
            get(move || {
                let stats = stats.clone();
                async move {
                    let s = stats.stats();
                    let streams: Vec<serde_json::Value> = s
                        .per_stream
                        .iter()
                        .map(|st| {
                            serde_json::json!({
                                "name": st.name,
                                "alive": st.alive,
                                "viewers": st.viewers,
                                "messagesSent": st.messages_sent,
                                "sendFailures": st.send_failures,
                            })
                        })
                        .collect();
                    let sessions: Vec<serde_json::Value> = s
                        .per_session
                        .iter()
                        .map(|se| {
                            serde_json::json!({
                                "id": se.session_id,
                                "peer": se.peer.to_string(),
                                "stream": se.stream_name,
                                "messagesPushed": se.messages_pushed,
                                "publishDropped": se.publish_dropped,
                            })
                        })
                        .collect();
                    let body = serde_json::json!({
                        "streams": streams,
                        "aliveStreams": s.alive_streams,
                        "totalViewers": s.total_viewers,
                        "activeSessions": s.active_sessions,
                        "sessions": sessions,
                    });
                    (
                        [
                            (header::CONTENT_TYPE, "application/json"),
                            (header::CACHE_CONTROL, "no-store"),
                        ],
                        body.to_string(),
                    )
                }
            }),
        )
        // Served dynamically: the WT cert hash changes whenever the identity
        // is regenerated, so it must never be cached as a static asset.
        .route(
            "/cert-hash.js",
            get(move || {
                let js = cert_hash_js.clone();
                async move {
                    (
                        [
                            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
                            (header::CACHE_CONTROL, "no-store"),
                        ],
                        js,
                    )
                }
            }),
        )
        .fallback(serve_web_file)
}

/// Proxy a remote gateway's `cert-hash.js` (embedding.md "Delivering the
/// cert hash cross-origin", pattern copied from SlopShady's
/// /api/stream/cert-hash). The caller passes the gateway's WEB URL — the
/// page the user browses to; cert-hash.js is served same-origin there. One
/// fetch yields both CERT_HASH (hex for self-signed pinning, null for
/// PKI/mkcert) and WT_PORT, which is everything the browser needs to build
/// the WT URL and pin the cert.
///
/// `danger_accept_invalid_certs(true)` is acceptable here because the real
/// trust anchor is the WebTransport `serverCertificateHashes` pinning done
/// client-side from the hash this proxy returns — TLS is just a transport
/// for the hash bytes, not the trust root.
#[derive(serde::Deserialize)]
struct CertHashParams {
    url: String,
}

async fn api_cert_hash(
    axum::extract::Query(params): axum::extract::Query<CertHashParams>,
) -> Response {
    let cert_url = match validate_gateway_target(&params.url) {
        Ok(u) => u,
        Err(e) => return cert_hash_json(None, None, Some(&e)),
    };
    let client = match reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
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
/// cert-hash.js. Returns the cert-hash.js URL to fetch.
///
/// The endpoint exists because a browser cannot fetch another origin's
/// cert-hash.js (CORS) — legitimate targets are REMOTE WebSRT gateways.
/// That also makes it an open prober for internal networks unless guarded:
/// - scheme must be https (gateways are always TLS; WT requires it)
/// - hostnames that ARE IP literals must not be loopback / private
///   (RFC1918 + CGNAT) / link-local / unspecified
/// - localhost / *.local / *.internal names rejected
///
/// Residual risk, accepted by design: DNS rebinding (the validator checks
/// the NAME; resolution happens inside reqwest) and the fact that the hash
/// bytes themselves are not the trust anchor — client-side
/// `serverCertificateHashes` pinning is. TLS validation is deliberately off
/// (`danger_accept_invalid_certs(true)`): self-signed gateway certs are the
/// norm here.
fn validate_gateway_target(raw: &str) -> Result<String, String> {
    let parsed = url::Url::parse(raw).map_err(|_| "invalid gateway url".to_string())?;
    if parsed.scheme() != "https" {
        return Err(format!(
            "gateway url must be https, got {}",
            parsed.scheme()
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "gateway url has no host".to_string())?;
    let port = parsed.port_or_known_default().unwrap_or(443);

    // IP literals get a numeric classification; named hosts get suffix rules.
    // (Parsing first matters: "10.0.0.5.example.com" is a NAME, not the
    // private literal 10.0.0.5.)
    // url serializes IPv6 hosts WITH brackets ("[::1]"), so strip them before
    // attempting IpAddr parse.
    let bare_host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = bare_host.parse::<std::net::IpAddr>() {
        let banned = match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.is_broadcast()
                    // CGNAT 100.64.0.0/10 has no is_* predicate on Ipv4Addr.
                    || v4.octets()[0] == 100 && (v4.octets()[1] & 0b1100_0000) == 0b0100_0000
            }
            std::net::IpAddr::V6(v6) => {
                // No is_private on Ipv6Addr: loopback (::1) and unspecified
                // (::) cover the local cases; unique-local fc00::/7 tested
                // directly (!is_global would over-reject documentation etc.).
                v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00) == 0xfc00
            }
        };
        if banned {
            return Err("gateway url points at a non-public address".to_string());
        }
    } else {
        let lower = host.to_ascii_lowercase();
        if lower == "localhost" || lower.ends_with(".local") || lower.ends_with(".internal") {
            return Err(format!("gateway host {host:?} is not allowed"));
        }
    }

    Ok(format!("https://{host}:{port}/cert-hash.js"))
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

/// cert-hash.js body, mirroring the websrt-gateway format: the DER SHA-256 hex
/// of the WT identity (or null when the browser should use normal PKI), plus
/// the actual WT port.
fn build_cert_hash_js(cert: &Cert, wt_port: u16) -> String {
    match cert.der_sha256.as_ref() {
        Some(hash) => {
            tracing::info!("WebTransport cert DER SHA-256: {}", hex::encode(hash));
            format!(
                "window.CERT_HASH = \"{}\";\nwindow.WT_PORT = {};",
                hex::encode(hash),
                wt_port
            )
        }
        None => {
            tracing::info!("PEM identity loaded; browser uses normal PKI");
            format!("window.CERT_HASH = null;\nwindow.WT_PORT = {wt_port};")
        }
    }
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

    // quinn (via websrt/wtransport) links the ring rustls provider while
    // axum-server's rustls default is aws-lc-rs; with both in the process
    // rustls cannot pick a default by itself. Install ring explicitly (same
    // as the websrt-gateway reference binary).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cli = Cli::parse();

    let mode = match (cli.cert_pem.clone(), cli.key_pem.clone()) {
        (Some(cert), Some(key)) => CertMode::Pem { cert, key },
        (None, None) => CertMode::SelfSigned,
        _ => unreachable!("clap `requires` enforces the PEM flag pair"),
    };

    // WebTransport identity — always needed, even with --no-tls (WT runs its
    // own TLS). Self-signed identities are regenerated on every boot:
    // serverCertificateHashes pinning caps cert validity at 14 days, so
    // persisting can't extend it (vendor/WebSRT/docs/embedding.md "Cert
    // modes"); /cert-hash.js always serves the fresh hash.
    let wt_cert_source = match &mode {
        CertMode::SelfSigned => CertSource::SelfSigned {
            sans: vec![
                "localhost".to_string(),
                "127.0.0.1".to_string(),
                "::1".to_string(),
            ],
        },
        CertMode::Pem { cert, key } => CertSource::Mkcert {
            cert: cert.clone(),
            key: key.clone(),
        },
    };
    let wt_cert = Cert::build(wt_cert_source).await.unwrap_or_else(|e| {
        eprintln!("Error: failed to build WebTransport identity: {e}");
        std::process::exit(1);
    });

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

    let cert_hash_js = build_cert_hash_js(&wt_cert, cli.wt_port);
    // Same identity/port advertised to the reference web UI on :8201 — both
    // servers front the SAME gateway.
    let ref_cert_hash_js = cert_hash_js.clone();

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
        "  webrt:      udp 0.0.0.0:{} (latency {} ms)",
        cli.wt_port, cli.latency_ms
    );
    println!(
        "  srt ingest: udp 0.0.0.0:{} → stream \"{}\"",
        cli.srt_port,
        gateway::SRT_STREAM_NAME
    );
    match &mode {
        CertMode::SelfSigned => println!(
            "  cert mode:  self-signed (web PEM persisted; WT identity ≤14d, hash at /cert-hash.js)"
        ),
        CertMode::Pem { cert, .. } => println!(
            "  cert mode:  pem {} (browser PKI validation; CERT_HASH=null)",
            cert.display()
        ),
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

    let (gateway_task, gateway_stats) = gateway::spawn(
        wt_cert,
        cli.wt_port,
        cli.srt_port,
        cli.latency_ms,
        shutdown.clone(),
    )
    .unwrap_or_else(|e| {
        eprintln!("Error: failed to build WebTransport gateway: {e}");
        std::process::exit(1);
    });

    // Open browser after a short delay.
    let open_url = url.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let _ = open::that(&open_url);
    });

    let axum_task = spawn_web_server(
        addr,
        build_router(cert_hash_js, std::sync::Arc::new(gateway_stats)),
        web_tls.clone(),
        shutdown.clone(),
    )
    .await
    .unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        std::process::exit(1);
    });

    // Reference WebSRT web UI on its own HTTPS port (vendor demo app — the
    // canonical viewer/publisher pages for our gateway). Requires web TLS
    // material; skipped when disabled (--web-port 0) or in --no-tls dev mode.
    let ref_web_task: Option<tokio::task::JoinHandle<anyhow::Result<()>>> = if cli.web_port != 0 {
        match &web_tls {
            Some((cert_pem, key_pem)) => Some(tokio::spawn(ref_web::run(
                cli.web_bind.clone(),
                cli.web_port,
                ref_cert_hash_js,
                cert_pem.clone(),
                key_pem.clone(),
                shutdown.clone(),
            ))),
            None => {
                println!("  ref web:    disabled (--no-tls)");
                None
            }
        }
    } else {
        None
    };

    // Exit on ctrl-c, or as soon as any server task fails.
    tokio::pin!(axum_task);
    tokio::pin!(gateway_task);

    enum Exited {
        CtrlC,
        WebServer,
        Gateway,
        RefWeb,
    }
    let exited = tokio::select! {
        _ = shutdown.notified() => Exited::CtrlC,
        res = &mut axum_task => {
            if let Ok(Err(e)) = res {
                eprintln!("Error: web server failed: {e}");
            }
            Exited::WebServer
        }
        res = &mut gateway_task => {
            if let Ok(Err(e)) = res {
                eprintln!("Error: WebTransport gateway failed: {e}");
            }
            Exited::Gateway
        }
        res = async {
            match ref_web_task {
                Some(t) => t.await.map_err(anyhow::Error::from).and_then(|r| r),
                None => std::future::pending().await,
            }
        } => {
            if let Err(e) = res {
                eprintln!("Error: reference web server failed: {e}");
            }
            Exited::RefWeb
        }
    };
    if let Exited::CtrlC = exited {
        println!("Shutting down.");
    }
    shutdown.notify_waiters();

    // Drain: the axum task exits via its shutdown watcher/handle, the gateway
    // drains its sessions when `shutdown` fires. Tasks that already finished
    // (the select! winner) are not awaited again.
    if !matches!(exited, Exited::WebServer) {
        let _ = (&mut axum_task).await;
    }
    if !matches!(exited, Exited::Gateway) {
        let _ = (&mut gateway_task).await;
    }
}

#[cfg(test)]
mod tests {
    use super::validate_gateway_target;

    #[test]
    fn accepts_public_ip_literal() {
        let url = validate_gateway_target("https://203.0.113.7:8201/").unwrap();
        assert_eq!(url, "https://203.0.113.7:8201/cert-hash.js");
    }

    #[test]
    fn accepts_public_dns_name_and_preserves_port_and_path() {
        // Port comes from the URL (no explicit port → https default 443);
        // query strings are dropped — only the cert-hash.js path matters.
        let url = validate_gateway_target("https://mixer.example.com:5173/?stream=x").unwrap();
        assert_eq!(url, "https://mixer.example.com:5173/cert-hash.js");
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
    fn rejects_private_and_local_ip_literals() {
        for raw in [
            "https://127.0.0.1/",
            "https://[::1]/",
            "https://10.0.0.5/",
            "https://192.168.1.214:5173/",
            "https://172.16.0.9/",
            "https://169.254.1.1/",
            "https://100.64.0.1/",    // CGNAT
            "https://100.127.255.9/", // CGNAT top edge
            "https://0.0.0.0/",
            "https://[fd00::5]/", // IPv6 unique-local
            "https://[::]/",      // unspecified v6
        ] {
            let err = validate_gateway_target(raw).expect_err(&format!("{raw} must be rejected"));
            assert!(err.contains("non-public"), "{raw}: got {err}");
        }
    }

    #[test]
    fn rejects_internal_hostnames() {
        for raw in [
            "https://localhost:4433/",
            "https://LOCALHOST/", // case-insensitive
            "https://gateway.local/",
            "https://foo.internal/",
        ] {
            let err = validate_gateway_target(raw).expect_err(&format!("{raw} must be rejected"));
            assert!(err.contains("not allowed"), "{raw}: got {err}");
        }
    }

    #[test]
    fn allows_named_hosts_that_merely_look_like_ips() {
        // A NAME that embeds a private IP is not that IP — suffix rules apply.
        assert!(validate_gateway_target("https://10.0.0.5.nip.io/").is_ok());
    }
}
