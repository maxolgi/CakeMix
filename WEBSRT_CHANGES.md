# WebSRT Changes Needed for PCM-Only MPTS

CakeMix transports raw linear PCM audio inside MPEG2-TS over WebSRT/SRT
using SMPTE 302M (s302m) stream type. No codec encode/decode — PCM bytes
go straight into TS PES packets.

## PCM transport contract (confirmed with user)

- PCM arrives as **Float32 interleaved** per PID (i32→f32 done in demuxer)
- **48 kHz fixed** — resampling is the mixer's concern
- **ptsMs from PES PTS** — ffmpeg populates correctly for s302m
- **PID map can change mid-stream** — mixer handles pidmap events idempotently
- Stream type: s302m (not the generic private 0x06 we initially proposed)

The current `ts-muxer-wasm` is hardcoded for Opus audio + H.264/AV1/HEVC
video. The changes below make it support audio-only PCM streams and
multi-program TS (MPTS).

**All changes are in `crates/ts-muxer-wasm/src/lib.rs` unless noted.**

---

## Change 1: Add LPCM stream type and descriptor

### Context
Currently:
```rust
const STREAM_TYPE_OPUS: u8 = 0x06;
const OPUS_DESCRIPTOR: &[u8] = &[0x05, 0x04, 0x4F, 0x70, 0x75, 0x73, ...];
```

### Add
```rust
/// LPCM (linear PCM) as a private stream with registration descriptor.
/// Stream type 0x06 = "private section" (same as Opus-in-TS convention).
const STREAM_TYPE_S302M: u8 = 0x06;
const S302M_DESCRIPTOR: &[u8] = &[
    0x05, 0x04, 0x43, 0x55, 0x45, 0x53, // registration "CUES" (SMPTE 302M)
];
```

---

## Change 2: Audio codec selection (constructor or setter)

### Context
Currently audio is always Opus. The muxer needs to know if audio is PCM.

### Add a setter (mirrors `set_video_codec`)
```rust
#[wasm_bindgen(js_name = setAudioCodec)]
pub fn set_audio_codec(&mut self, codec: &str) {
    match codec {
        "s302m" => {
            self.audio_stream_type = STREAM_TYPE_S302M;
            self.audio_descriptor = LPCM_DESCRIPTOR.to_vec();
        }
        _ => {
            // default: Opus
            self.audio_stream_type = STREAM_TYPE_OPUS;
            self.audio_descriptor = OPUS_DESCRIPTOR.to_vec();
        }
    }
}
```

### Add fields to `TsMuxer` struct
```rust
audio_stream_type: u8,    // default STREAM_TYPE_OPUS
audio_descriptor: Vec<u8>, // default OPUS_DESCRIPTOR
```

### Update `write_pmt()` to use the configured audio type
Replace the hardcoded `STREAM_TYPE_OPUS` and `OPUS_DESCRIPTOR` in
`write_pmt()` with `self.audio_stream_type` and `self.audio_descriptor`.

---

## Change 3: `push_audio` — don't prepend Opus header for PCM

### Context (lines ~115-127)
```rust
pub fn push_audio(&mut self, data: &[u8], pts_us: f64) {
    let pts_90k = us_to_90k(pts_us);
    // Prepend the 2-byte Opus-in-TS control header (ffmpeg convention).
    let mut payload = Vec::with_capacity(2 + data.len());
    payload.extend_from_slice(&[0x7F, 0xE0]);
    payload.extend_from_slice(data);
    ...
}
```

### Change
When audio is PCM, push the raw bytes without the Opus header:
```rust
pub fn push_audio(&mut self, data: &[u8], pts_us: f64) {
    let pts_90k = us_to_90k(pts_us);
    let pes = if self.audio_stream_type == STREAM_TYPE_S302M {
        // PCM: raw bytes, no codec framing
        build_pes_audio(data, pts_90k)
    } else {
        // Opus: prepend 2-byte control header (ffmpeg convention)
        let mut payload = Vec::with_capacity(2 + data.len());
        payload.extend_from_slice(&[0x7F, 0xE0]);
        payload.extend_from_slice(data);
        build_pes_audio(&payload, pts_90k)
    };
    packetize(&mut self.output, self.audio_pid, &mut self.audio_cc, &pes, None, false);
}
```

---

## Change 4: Audio-only mode — emit PAT/PMT without video

### Context (lines ~70-98)
Currently `pat_pmt_emitted` is set `true` only inside `push_video()`
on keyframes. If there's no video, PAT/PMT are never emitted.

### Change
Add a flag for audio-only operation. When `push_audio` is called and
`pat_pmt_emitted` is false, emit PAT/PMT:

```rust
pub fn push_audio(&mut self, data: &[u8], pts_us: f64) {
    if !self.pat_pmt_emitted {
        self.write_pat();
        self.write_pmt();
        self.pat_pmt_emitted = true;
    }
    // ... rest of push_audio
}
```

Also: when audio-only, the PCR PID must be the audio PID.

### Add PCR PID configuration
```rust
#[wasm_bindgen(js_name = setPcrPid)]
pub fn set_pcr_pid(&mut self, pid: u16) {
    self.pcr_pid = pid;
}
```

Default `pcr_pid` to video PID (0x100). In audio-only mode, set it to
the audio PID (0x101) or the designated program's audio PID.

Update `write_pmt()` to use `self.pcr_pid` instead of hardcoded 0x100.
Update `push_video()` and `push_audio()` PCR adaptation field to use
`self.pcr_pid`.

---

## Change 5: MPTS — multi-program support

### Context
Currently the muxer handles a single program (program_number=1) with
fixed video PID 0x100 and audio PID 0x101.

### For M1: not needed (single program).
### For M2: add multi-program API.

This is a larger change (multiple PAT entries, multiple PMTs, per-program
PIDs). Design deferred to M2 when we actually need multiple programs.
The single-program changes above are sufficient for M1.

---

## Demuxer side (mpeg2ts-wasm) — no changes needed

The demuxer already:
- Parses PMT entries per PID with stream_type
- Emits raw PES payload bytes per PID via `TsEvent::pes`
- Does NOT strip codec headers (no Opus 2-byte strip on demux)
- Reports registration descriptors ("Opus", "AV01", etc.)

For PCM, the demuxer will see stream_type 0x06 with "LPCM" registration.
The CakeMix JS layer reads the PES payload directly as PCM bytes.

---

## Summary of edits

| File | Change | Lines affected |
|------|--------|---------------|
| `ts-muxer-wasm/src/lib.rs` | Add LPCM constants | after line 30 |
| `ts-muxer-wasm/src/lib.rs` | Add `audio_stream_type`/`audio_descriptor` fields | struct def (~line 28) |
| `ts-muxer-wasm/src/lib.rs` | Add `set_audio_codec()` setter | after `set_video_codec` |
| `ts-muxer-wasm/src/lib.rs` | `push_audio`: conditional Opus header | lines 115-127 |
| `ts-muxer-wasm/src/lib.rs` | Audio-only PAT/PMT emission | in `push_audio` |
| `ts-muxer-wasm/src/lib.rs` | `set_pcr_pid()` + use in PMT/PCR | new method + write_pmt |
| `ts-muxer-wasm/src/lib.rs` | `write_pmt`: use configured audio type | lines ~165-185 |

None of these changes affect existing SlopShady usage (Opus defaults are
preserved). They are additive: new `set_audio_codec("s302m")` call activates
PCM mode; without it, behavior is unchanged.
