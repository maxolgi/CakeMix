# CakeMix ↔ WebSRT Integration Guide

## Architecture

```
[ffmpeg s302m] →SRT→ [WebSRT gateway] →WebTransport→ [WebSRT worker]
                                                          │
                                                pcm-port (transferred MessagePort,
                                                created by the store per connect)
                                                          │
                                             ┌────────────┴─────────────┐
                                             │  Mixer AudioWorklet      │
                                             │  - auto-maps PIDs on     │
                                             │    first pcm (packed     │
                                             │    from ch 0, cap 128)   │
                                             │  - WASM MixerWasm: feed, │
                                             │    elastic FIFOs, gain/  │
                                             │    pan/EQ/dynamics       │
                                             └────────────┬─────────────┘
                                                          │
                                        {type:'pid-mapped' / 'pcm-dropped'}
                                                          │
                                             ┌────────────┴─────────────┐
                                             │  CakeMix main thread     │
                                             │  - mirrors mapping for   │
                                             │    the UI (strips,       │
                                             │    meters, drawer)       │
                                             │  (no PCM on this thread) │
                                             └──────────────────────────┘
```

## PCM Handoff Contract

Raw pcm flows **worker → worklet over a transferred `MessagePort`** (zero
main-thread hops; vendor/WebSRT `docs/embedding.md` "pcm-port"). The store
creates a fresh `MessageChannel` per connect, transfers port1 to the worker
(`{cmd:'pcm-port', port}`) and port2 to the worklet
(`{type:'pcm-port', port}`); a fresh channel per connect is required
because the worker is recreated on reconnect and transferred ports die
with it.

Worker → worklet, on the port (batched, sample buffers transferred — see
`vendor/WebSRT/web/src/worker.ts` `flushOutgoing`):
```ts
{ type: 'batch', msgs: [{ type: 'pcm', pid: number, channelCount: number,
    samples: Float32Array, pts: number | null, schedUs?: number, relUs?: number }] }
```

Worklet → main (events, on the worklet's own port):
```js
{ type: 'pid-mapped', pid, chStart, channelCount }  // chStart -1 = capped at 128 ch
{ type: 'pcm-dropped', total }                       // cumulative (wasm not ready / cap)
```

The parent-channel relay (`{type:'pcm', pid, samples}` → worklet) remains
as a fallback for when the port is not wired (e.g. mid-handshake); it runs
the identical worklet-side auto-mapping and is counted in
`websrtRelayPcm()` — 0 in normal operation.

## PID Mapping

Mapping is triggered by the **first pcm per PID** (not the PMT — the
channelCount carried there is authoritative, auto-detected from the AES3
frame header by the WebSRT demuxer; see `mpeg2ts-wasm/src/aes3.rs`). It
executes worklet-side (see `web/worklet-template.js` `_onPcm`) so the
direct port path needs no main-thread round trip:

- PIDs are packed consecutively from mixer channel 0.
- Capped at 128 channels total (AGENTS.md "128 input strips max");
  overflow PIDs report `chStart: -1` via `pid-mapped` and their PCM is
  dropped (counted).
- The store mirrors the mapping for the UI from `pid-mapped` events
  (`frontend/src/websrt/store.ts`).
- `map_pid` also (re)anchors the channel's elastic playout FIFOs — a
  remap onto the same channels is a new stream.

For multi-PID audio (CakeMix use case), the PMT entries from the demuxer
(`TsEvent` kind=1) carry per-PID format IDs. PIDs with "BSSD" registration
are SMPTE 302M audio streams.

## Run ffmpeg Publisher

```bash
# Real-audio 30-track session (30 stereo s302m PIDs, looped):
./fixtures/stream_pcm_real.sh
```

Default SRT target: `srt://127.0.0.1:9000?streamid=audio`. Override with:
```bash
WEBSTRT_SRT_URL=srt://host:port?streamid=NAME ./fixtures/stream_pcm_real.sh
```

## No Compile-Time Dependency

CakeMix and WebSRT communicate purely through browser APIs:
- `MessagePort` (worker → worklet, PCM — transferred, zero-copy)
- `postMessage` (worker → main thread, control/stats)
- `AudioWorkletNode.port` (main thread → worklet, control + events)

Neither repo needs to import the other's WASM or Rust code. The WASM
modules (mixer-wasm + mpeg2ts-wasm) are loaded independently in the browser.

## Registration Descriptor

SMPTE 302M uses registration descriptor **"BSSD"** (0x42535344 =
"Broadcast Serial Sound Data"), per ffmpeg's s302m muxer. This is the
correct ID, not "CUES".
