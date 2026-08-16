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
use websrt::Gateway;

/// Stream name SRT input is published under (browsers subscribe via
/// `?stream=default`).
pub const SRT_STREAM_NAME: &str = "default";

/// Build the gateway, spawn the SRT ingest task, and return the running
/// `gateway.run()` task. The gateway exits (draining sessions) when
/// `shutdown` fires. Also returns a [`GatewayStatsHandle`] for the health/
/// stats HTTP endpoints (owned Arc clones; safe to use from other tasks).
pub fn spawn(
    wt_cert: Cert,
    wt_port: u16,
    srt_port: u16,
    latency_ms: u64,
    shutdown: Arc<Notify>,
) -> anyhow::Result<(JoinHandle<anyhow::Result<()>>, websrt::gateway::GatewayStatsHandle)> {
    let bind_addr: std::net::SocketAddr = format!("0.0.0.0:{wt_port}").parse()?;
    // Payload size needs no local override: since WebSRT 8da0d38 the
    // builder's `SrtConfig::default()` carries upstream `PAYLOAD_SIZE`
    // = 1128 (6 × 188, TS-aligned), which keeps browser→gateway datagrams
    // (1128 + 16 B SRT header) under Chrome's WebTransport datagram cap —
    // the exact fix our former `WT_PAYLOAD_SIZE` constant replicated.
    let gateway = Gateway::builder()
        .bind_addr(bind_addr)
        .identity(wt_cert.identity.clone_identity())
        .latency_ms(latency_ms)
        .build()?;

    let source = gateway.source_handle();
    let stats = gateway.stats_handle();

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

    Ok((
        tokio::spawn(async move {
            gateway.run(shutdown.notified()).await
        }),
        stats,
    ))
}
