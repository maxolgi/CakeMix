#!/bin/bash
# Real-audio source, 16-channel cut — pushes the FIRST 16 channels of the
# broadcast WAV (tmp/broadcast30.wav) over SRT to the running WebSRT
# gateway, packed 2 mono channels per stereo s302m PID (s302m has no mono
# layout): 16 distinct channels = 8 stereo PIDs, no duplication. The mixer
# unpacks each PID into individual mono strips (ch1..ch16 in channel order).
# 48 kHz, looped forever on a single shared loop point. Audio-only MPEG-2
# TS, no video.
#
# Usage: ./fixtures/stream_pcm_real16.sh
# Override source dir:  AUDIO_DIR=/path/to/wavs ./fixtures/stream_pcm_real16.sh
#   (only used to build the WAV if missing)
# Override target via WEBSTRT_SRT_URL (default srt://127.0.0.1:9000?streamid=audio).
# The gateway rejects SRT connections without a streamid.
# Ctrl+C to stop.

set -euo pipefail

AUDIO_DIR="${AUDIO_DIR:-$HOME/Downloads/audio/PawelMaciwoda_1OfTheFirst_Full}"
SRT_URL="${WEBSTRT_SRT_URL:-srt://127.0.0.1:9000?streamid=audio}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BROADCAST_WAV="$ROOT/tmp/broadcast30.wav"
MAX_CH=16

if ! ffmpeg -hide_banner -encoders 2>/dev/null | grep -qw s302m; then
  echo "error: this ffmpeg build does not include the s302m encoder (SMPTE 302M)." >&2
  echo "       s302m is GPL-only; install an ffmpeg built with --enable-gpl." >&2
  exit 1
fi

if [ ! -f "$BROADCAST_WAV" ]; then
  echo "broadcast WAV missing — building it (one-time, ~1.8 GB)..." >&2
  AUDIO_DIR="$AUDIO_DIR" OUT="$BROADCAST_WAV" "$ROOT/fixtures/make_broadcast_wav.sh"
fi

# Channel pairs -> stereo PIDs: PID k carries WAV channels 2k (L) + 2k+1 (R).
PANS=()
MAPS=()
META=()
SPLITS=()
for ((i = 0; 2 * i < MAX_CH; i++)); do
  PANS+=( "[s${i}]pan=stereo|c0=c$((2 * i))|c1=c$((2 * i + 1))[a${i}]" )
  MAPS+=( -map "[a${i}]" )
  META+=( -metadata:"s:a:${i}" "language=ch$((2 * i))-$((2 * i + 1))" )
  SPLITS+=( "[s${i}]" )
done

FC="[0:a]asplit=${i}$(IFS=''; echo "${SPLITS[*]}")"
FC+=";$(IFS=';'; echo "${PANS[*]}")"

echo "PCM: first ${MAX_CH} channels packed as ${i} stereo s302m PIDs from ${BROADCAST_WAV}" >&2
exec ffmpeg -re \
  -stream_loop -1 -i "$BROADCAST_WAV" \
  -filter_complex "$FC" \
  "${MAPS[@]}" \
  -c:a s302m -ac 2 -ar 48000 -sample_fmt s32 -strict -2 \
  "${META[@]}" \
  -f mpegts "$SRT_URL"
