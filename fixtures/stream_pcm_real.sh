#!/bin/bash
# Real-audio source — pushes the 30-track Pawel Maciwoda session over SRT to
# the running WebSRT gateway. Audio-only MPEG-2 TS, no video: one stereo s302m
# PID per track (mono tracks duplicated L=R — s302m has no mono layout), all
# resampled to 48 kHz (s302m requirement) and looped forever.
#
# Usage: ./fixtures/stream_pcm_real.sh
# Override source dir:  AUDIO_DIR=/path/to/wavs ./fixtures/stream_pcm_real.sh
# Override target via WEBSTRT_SRT_URL (default srt://127.0.0.1:9000?streamid=audio).
# The gateway rejects SRT connections without a streamid.
# Ctrl+C to stop.

set -euo pipefail

AUDIO_DIR="${AUDIO_DIR:-/home/flibb/Downloads/audio/PawelMaciwoda_1OfTheFirst_Full}"
SRT_URL="${WEBSTRT_SRT_URL:-srt://127.0.0.1:9000?streamid=audio}"

if ! ffmpeg -hide_banner -encoders 2>/dev/null | grep -qw s302m; then
  echo "error: this ffmpeg build does not include the s302m encoder (SMPTE 302M)." >&2
  echo "       s302m is GPL-only; install an ffmpeg built with --enable-gpl." >&2
  exit 1
fi

if [ ! -d "$AUDIO_DIR" ]; then
  echo "error: AUDIO_DIR does not exist: ${AUDIO_DIR}" >&2
  exit 1
fi

FFMPEG_INPUTS=()
FFMPEG_MAPS=()
FFMPEG_META=()
i=0
for f in "$AUDIO_DIR"/*.wav; do
  FFMPEG_INPUTS+=( -stream_loop -1 -i "$f" )
  FFMPEG_MAPS+=( -map "${i}:a" )
  FFMPEG_META+=( -metadata:"s:a:${i}" "language=ch${i}" )
  i=$((i + 1))
done

echo "PCM: ${i} tracks as ${i} stereo s302m PIDs from ${AUDIO_DIR}" >&2
exec ffmpeg -re \
  "${FFMPEG_INPUTS[@]}" \
  "${FFMPEG_MAPS[@]}" \
  -c:a s302m -ac 2 -ar 48000 -sample_fmt s32 -strict -2 \
  "${FFMPEG_META[@]}" \
  -f mpegts "$SRT_URL"
