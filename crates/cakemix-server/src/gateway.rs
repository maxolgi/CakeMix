//! WebTransport gateway + SRT ingest wiring.
//!
//! One UDP listener for WebTransport sessions (`0.0.0.0:<wt_port>`, driven by
//! `websrt::Gateway`) and one for SRT ingest (`0.0.0.0:<srt_port>`, an
//! `SrtIngester` publishing into the gateway under the stream name "default").
//! The SRT bind is deferred into its own task (the `lib.rs` quick-start
//! pattern) because `SrtIngester::bind` only returns once a publisher
//! connects — the UDP socket is bound long before that, inside the task.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::JoinHandle;
use websrt::cert::Cert;
use websrt::ingest::srt::SrtIngester;
use websrt::srt_sender::SrtConfig;
use websrt::Gateway;

/// Stream name SRT input is published under (browsers subscribe via
/// `?stream=default`).
pub const SRT_STREAM_NAME: &str = "default";

/// SRT payload size for browser (WebTransport) sessions. The SRT default
/// (1316 + 16-byte header = 1332 B on the wire) exceeds what Chrome will put
/// in a single QUIC packet (~1350 B including QUIC framing) on the
/// browser→gateway leg, so every full-size data datagram is silently dropped
/// — the handshake (small control packets) still succeeds, which makes it
/// look alive while no data flows. HSv5 negotiates `min(local, peer)`, so
/// advertising 1128 (6 × 188, TS-aligned) caps the browser's sender at
/// 1128 + 16 = 1144 B per datagram, which fits. Verified empirically against
/// wtransport 0.7.1 + Chrome: 1200 B datagrams pass, 1332 B vanish.
pub const WT_PAYLOAD_SIZE: u64 = 1128;

/// Build the gateway, spawn the SRT ingest task, and return the running
/// `gateway.run()` task. The gateway exits (draining sessions) when
/// `shutdown` fires.
pub fn spawn(
    wt_cert: Cert,
    wt_port: u16,
    srt_port: u16,
    latency_ms: u64,
    shutdown: Arc<Notify>,
) -> anyhow::Result<JoinHandle<anyhow::Result<()>>> {
    let bind_addr: std::net::SocketAddr = format!("0.0.0.0:{wt_port}").parse()?;
    let gateway = Gateway::builder()
        .bind_addr(bind_addr)
        .identity(wt_cert.identity.clone_identity())
        .srt_config(SrtConfig {
            payload_size: WT_PAYLOAD_SIZE,
            ..SrtConfig::default()
        })
        .latency_ms(latency_ms)
        .build()?;

    let source = gateway.source_handle();

    let srt_addr = format!("0.0.0.0:{srt_port}");
    tokio::spawn(async move {
        tracing::info!(
            addr = %srt_addr,
            stream = SRT_STREAM_NAME,
            "SRT ingest listening; stream goes live when a publisher connects"
        );
        match SrtIngester::bind(&srt_addr, None, Duration::from_millis(latency_ms), None).await {
            Ok(ingester) => {
                tracing::info!(
                    stream = SRT_STREAM_NAME,
                    "SRT publisher connected; publishing stream"
                );
                source.publish_stream(SRT_STREAM_NAME, ingester);
            }
            Err(e) => {
                tracing::error!(?e, addr = %srt_addr, "SRT ingest bind failed");
            }
        }
    });

    Ok(tokio::spawn(async move {
        gateway.run(shutdown.notified()).await
    }))
}
