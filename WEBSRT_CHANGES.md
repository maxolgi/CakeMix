# WebSRT Changes for PCM — COMPLETED ✅

All changes documented below have been IMPLEMENTED in WebSRT
(https://github.com/maxolgi/WebSRT) as of commit a4181b1.

CakeMix's M1 blocker is RESOLVED. The integration is browser-only:
WebSRT's worker emits {type:'pcm'} messages; CakeMix's mixer worklet
receives them via port.postMessage. No compile-time dependency.

## What was implemented

| Change | Status | Details |
|--------|--------|---------|
| S302M stream type + descriptor | ✅ | Registration ID "BSSD" (correct per ffmpeg) |
| setAudioCodec() setter | ✅ | setAudioCodec("s302m", channel_count) |
| push_pcm() without Opus header | ✅ | New method: AES3 wraps via aes3 module |
| Audio-only PAT/PMT emission | ✅ | push_pcm emits PAT/PMT when needed |
| Multi-PID support | ✅ | addAudioPid(pid, codec, channel_count) |
| PCR PID auto-config | ✅ | Audio PID when video disabled |
| AES3 encode (muxer) | ✅ | aes3::wrap_smpte302m_pes(f32, ch, 24-bit) |
| AES3 decode (demuxer) | ✅ | aes3::unwrap_smpte302m_pes(payload) → Vec<f32> |
| Demuxer PCM events | ✅ | TsEvent kind=5: pcm(pid, pts, ch_count, samples) |
| Browser PCM playback | ✅ | PcmPlayer AudioWorklet ring buffer |
| ffmpeg s302m publisher | ✅ | fixtures/stream_pcm.sh [mono\|stereo\|surround] |
| audioplan.md transport plan | ✅ | Shared between WebSRT and CakeMix |

## Test status
- WebSRT workspace: 145 passed, 0 failed
- mpeg2ts-wasm: 24 tests (incl. AES3 round-trip)
- ts-muxer-wasm: 9 tests (incl. S302M descriptor, multi-PID, audio-only)
- WASM builds clean for both crates

## Registration ID correction
Our original spec said "CUES" (0x43554553). The correct SMPTE 302M
registration descriptor per ffmpeg's s302m muxer is "BSSD" (0x42535344)
= "Broadcast Serial Sound Data". WebSRT uses BSSD.

## Integration path (browser-only)
```
[ffmpeg s302m] →SRT→ [gateway] →WebTransport→ [worker.ts]
                                                     │
                                        {type:'pcm', pid,
                                         channelCount, samples, pts}
                                                     │
                                          CakeMix MixerWasm AudioWorklet
                                          - map_pid(pid, ch_start, nch)
                                          - feed_pcm(pid, samples)
                                          - process() → stereo out
```

The WebSRT worker already emits per-PID PCM events with channel_count.
CakeMix replaces WebSRT's PcmPlayer with the full mixer.
