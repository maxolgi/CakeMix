# CakeMix ↔ WebSRT Integration Guide

## Architecture

```
[ffmpeg s302m] →SRT→ [WebSRT gateway] →WebTransport→ [WebSRT worker]
                                                          │
                                                {type:'pcm', pid,
                                                 channelCount,
                                                 samples, pts}
                                                          │
                                            ┌─────────────┴──────────────┐
                                            │  CakeMix main thread (app.js)
                                            │  - Receives PCM from WebSRT worker
                                            │  - Relays to mixer AudioWorklet
                                            │  - UI: channel strips, meters
                                            └─────────────────────────────┘
                                                          │
                                            port.postMessage({type:'pcm', pid, samples})
                                                          │
                                            ┌─────────────┴──────────────┐
                                            │  Mixer AudioWorklet
                                            │  - WASM MixerWasm processes
                                            │  - gain/pan/EQ/dynamics
                                            │  - stereo output
                                            └─────────────────────────────┘
```

## PCM Handoff Contract

WebSRT worker emits (see `web/src/worker.ts`):
```ts
{ type: 'pcm', pid: number, channelCount: number, samples: Float32Array, pts: number | null }
```

CakeMix mixer worklet receives (see `web/worklet-template.js`):
```js
port.onmessage = (e) => {
    if (e.data.type === 'pcm') {
        mixer.feed_pcm(e.data.pid, e.data.samples);
    }
};
```

## PID Mapping

On PMT events, WebSRT worker emits (see `web/src/worker.ts`):
```ts
{ type: 'pmt', videoPid: number, audioPid: number, audioStreamType: number, videoCodec: ... }
```

For multi-PID audio (CakeMix use case), the PMT entries from the demuxer
(`TsEvent` kind=1) carry per-PID format IDs. PIDs with "BSSD" registration
are SMPTE 302M audio streams.

CakeMix maps each audio PID to consecutive mixer channels:
```js
// For each BSSD PID in the PMT:
sendToWorklet({
    type: 'map-pid',
    pid: pid,
    chStart: nextChannelStart,
    channelCount: channelCount
});
nextChannelStart += channelCount;
```

## Integration Steps

### 1. Replace WebSRT's PcmPlayer with CakeMix Mixer

In WebSRT's `viewer.ts`, replace the `PcmPlayer` instantiation with a
CakeMix mixer AudioWorklet connection:

```ts
// Before (WebSRT standalone):
pcmPlayer = new PcmPlayer();

// After (with CakeMix):
// Create a MessagePort bridge between the WebSRT worker and the CakeMix worklet.
// The main thread relays PCM messages.
```

### 2. Bridge PCM Events

The WebSRT worker posts PCM to the main thread. The main thread relays
to the CakeMix worklet:

```ts
case 'pcm': {
    // Feed PCM to CakeMix mixer worklet
    mixerNode.port.postMessage({
        type: 'pcm',
        pid: msg.pid,
        samples: msg.samples
    }, [msg.samples.buffer]); // transferable, zero-copy
    break;
}
```

### 3. Channel Count Discovery

The WebSRT demuxer auto-detects channel count from the AES3 frame header
(see `mpeg2ts-wasm/src/aes3.rs`). The PCM event carries `channelCount`
in `event.program_num`. Use this to configure mixer channel mapping.

### 4. Run ffmpeg Publisher

```bash
# Mono (440 Hz sine, duplicated to stereo pair for AES3)
./fixtures/stream_pcm.sh mono

# Stereo (440/660 Hz pair)
./fixtures/stream_pcm.sh stereo

# 5.1 surround (6-channel sine bed)
./fixtures/stream_pcm.sh surround
```

Default SRT target: `srt://127.0.0.1:9000`. Override with:
```bash
WEBSTRT_SRT_URL=srt://host:port ./fixtures/stream_pcm.sh stereo
```

## No Compile-Time Dependency

CakeMix and WebSRT communicate purely through browser APIs:
- `postMessage` (worker → main thread)
- `AudioWorkletNode.port` (main thread → worklet)

Neither repo needs to import the other's WASM or Rust code. The WASM
modules (mixer-wasm + mpeg2ts-wasm) are loaded independently in the browser.

## Registration Descriptor

SMPTE 302M uses registration descriptor **"BSSD"** (0x42535344 =
"Broadcast Serial Sound Data"), per ffmpeg's s302m muxer. This is the
correct ID, not "CUES".
