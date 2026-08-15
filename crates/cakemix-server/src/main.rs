mod certs;
mod gateway;

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
/// Paths are relative to the workspace root.
#[derive(Embed)]
#[folder = "/home/flibb/CakeMix/web"]
#[prefix = "web/"]
struct WebAsset;

#[derive(Embed)]
#[folder = "/home/flibb/CakeMix/crates/mixer-wasm/pkg"]
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

    #[arg(long, default_value_t = 9000u16, help = "SRT ingest UDP port")]
    srt_port: u16,

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

fn build_router(cert_hash_js: String) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/health", get(health))
        // Served dynamically: the WT cert hash changes whenever the identity
        // is regenerated, so it must never be cached as a static asset.
        .route(
            "/cert-hash.js",
            get(move || {
                let js = cert_hash_js.clone();
                async move {
                    (
                        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                        js,
                    )
                }
            }),
        )
        .fallback(serve_web_file)
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

    let gateway_task = gateway::spawn(
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

    let axum_task = spawn_web_server(addr, build_router(cert_hash_js), web_tls, shutdown.clone())
        .await
        .unwrap_or_else(|e| {
            eprintln!("Error: {e}");
            std::process::exit(1);
        });

    // Exit on ctrl-c, or as soon as either server task fails.
    tokio::pin!(axum_task);
    tokio::pin!(gateway_task);

    enum Exited {
        CtrlC,
        WebServer,
        Gateway,
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
