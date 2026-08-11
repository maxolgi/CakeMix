use std::net::SocketAddr;
use std::path::PathBuf;

use axum::http::header;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use clap::Parser;
use mime_guess::from_path;
use rust_embed::Embed;

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

#[derive(Parser)]
#[command(name = "cakemix", about = "CakeMix — WASM Audio Mixer")]
struct Cli {
    #[arg(short, long, default_value = "8200")]
    port: u16,

    #[arg(long, default_value = "0.0.0.0", help = "Bind address")]
    bind: String,

    #[arg(long, help = "Use HTTP instead of HTTPS")]
    no_tls: bool,
}

async fn serve_index() -> Response {
    match WebAsset::get("web/index.html") {
        Some(content) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8"),
             (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")],
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
            [(header::CONTENT_TYPE, mime.as_ref()),
             (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")],
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
            [(header::CONTENT_TYPE, mime_str),
             (header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")],
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
        .fallback(serve_web_file)
}

fn ensure_cert(cert_path: &std::path::Path, key_path: &std::path::Path) {
    if !cert_path.exists() || !key_path.exists() {
        println!("Generating self-signed certificate for HTTPS...");
        use rcgen::{CertificateParams, DnType, Ia5String, KeyPair, SanType};
        use std::net::{IpAddr, Ipv4Addr};

        let mut params = CertificateParams::default();
        params.distinguished_name.push(DnType::CommonName, "localhost");
        params.subject_alt_names = vec![
            SanType::DnsName(Ia5String::try_from("localhost").unwrap()),
            SanType::IpAddress(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
        ];

        let key_pair = KeyPair::generate().unwrap();
        let cert = params.self_signed(&key_pair).unwrap();

        std::fs::write(cert_path, cert.pem()).unwrap();
        std::fs::write(key_path, key_pair.serialize_pem()).unwrap();
        println!("Certificate saved to: {}", cert_path.display());
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let bind_addr: std::net::IpAddr = cli
        .bind
        .parse()
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    let addr = SocketAddr::new(bind_addr, cli.port);

    let app = build_router();

    let url = if cli.no_tls {
        format!("http://localhost:{}", cli.port)
    } else {
        format!("https://localhost:{}", cli.port)
    };

    println!("CakeMix server starting on {url}");
    println!("Press Ctrl+C to stop.");

    // Open browser after a short delay.
    let open_url = url.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let _ = open::that(&open_url);
    });

    if cli.no_tls {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .unwrap_or_else(|e| {
                eprintln!("Error: could not bind to {addr}: {e}");
                std::process::exit(1);
            });
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    } else {
        let cert_path = PathBuf::from("cert.pem");
        let key_path = PathBuf::from("key.pem");
        ensure_cert(&cert_path, &key_path);

        let tls_config =
            axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert_path, &key_path)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("Error: failed to load TLS certificate: {e}");
                    std::process::exit(1);
                });

        axum_server::bind_rustls(addr, tls_config)
            .serve(app.into_make_service())
            .await
            .unwrap_or_else(|e| {
                eprintln!("Error: could not bind to {addr}: {e}");
                std::process::exit(1);
            });
    }
}
