//! Reference WebSRT web UI server — the vendor/WebSRT/web demo app (viewer,
//! publisher, debug panels) embedded at compile time and served on its own
//! HTTPS port, mirroring websrt-gateway's web_server.rs. The point: the
//! canonical consumers of our gateway are the canonical pages — no custom
//! viewer to drift out of sync.
//!
//! `/cert-hash.js` is served dynamically (this server's gateway identity
//! changes across restarts) and carries the MAIN gateway's WT port.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../vendor/WebSRT/web/dist"]
struct RefWebAsset;

/// Run the HTTPS server for the reference web UI. Blocks until `shutdown`
/// fires; the caller spawns this as a background task.
pub async fn run(
    bind: String,
    port: u16,
    cert_hash_js: String,
    cert_pem: Vec<u8>,
    key_pem: Vec<u8>,
    shutdown: Arc<tokio::sync::Notify>,
) -> Result<()> {
    let addr: SocketAddr = format!("{bind}:{port}")
        .parse()
        .context("invalid reference web server bind address")?;

    let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem(cert_pem, key_pem)
        .await
        .context("failed to build TLS config for reference web server")?;

    let app = build_router(cert_hash_js);
    let handle = axum_server::Handle::new();

    let handle_clone = handle.clone();
    let shutdown_clone = shutdown.clone();
    tokio::spawn(async move {
        shutdown_clone.notified().await;
        handle_clone.shutdown();
    });

    tracing::info!(%addr, "reference web UI (WebSRT demo app) starting");
    axum_server::bind_rustls(addr, tls_config)
        .handle(handle)
        .serve(app.into_make_service())
        .await
        .context("reference web server stopped with error")?;
    Ok(())
}

fn build_router(cert_hash_js: String) -> Router {
    Router::new()
        .route(
            "/cert-hash.js",
            get(move || {
                let js = cert_hash_js.clone();
                async move {
                    (
                        [
                            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
                            // Dynamic identity — must never be cached.
                            (header::CACHE_CONTROL, "no-store"),
                        ],
                        js,
                    )
                }
            }),
        )
        .fallback(serve_embedded)
}

/// Serve embedded static files from web/dist/. Falls back to index.html for
/// unknown paths (SPA-style), like the reference gateway's web server.
async fn serve_embedded(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match RefWebAsset::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref())],
                Body::from(content.data.into_owned()),
            )
                .into_response()
        }
        None => match RefWebAsset::get("index.html") {
            Some(content) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                Body::from(content.data.into_owned()),
            )
                .into_response(),
            None => (
                StatusCode::NOT_FOUND,
                "web UI not built — run ./build.sh web build in vendor/WebSRT",
            )
                .into_response(),
        },
    }
}
