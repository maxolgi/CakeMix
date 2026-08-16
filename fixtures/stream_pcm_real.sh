#!/bin/bash
# Real-audio source — pushes the 30-track Pawel Maciwoda session over SRT to
# the running WebSRT gateway. All tracks come from one sample-aligned
# multi-channel WAV (tmp/broadcast30.wav, built by make_broadcast_wav.sh if
# missing) and are split here into 30 stereo s302m PIDs — one PID per track,
# mono tracks duplicated L=R (s302m has no mono layout), 48 kHz, looped
# forever on a single shared loop point. Audio-only MPEG-2 TS, no video.
#
# Usage: ./fixtures/stream_pcm_real.sh
# Override source dir:  AUDIO_DIR=/path/to/wavs ./fixtures/stream_pcm_real.sh
# Override target via WEBSTRT_SRT_URL (default srt://127.0.0.1:9000?streamid=audio).
# The gateway rejects SRT connections without a streamid.
# Ctrl+C to stop.

set -euo pipefail

AUDIO_DIR="${AUDIO_DIR:-/home/flibb/Downloads/audio/PawelMaciwoda_1OfTheFirst_Full}"
SRT_URL="${WEBSTRT_SRT_URL:-srt://127.0.0.1:9000?streamid=audio}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BROADCAST_WAV="$ROOT/tmp/broadcast30.wav"

if ! ffmpeg -hide_banner -encoders 2>/dev/null | grep -qw s302m; then
  echo "error: this ffmpeg build does not include the s302m encoder (SMPTE 302M)." >&2
  echo "       s302m is GPL-only; install an ffmpeg built with --enable-gpl." >&2
  exit 1
fi

if [ ! -f "$BROADCAST_WAV" ]; then
  echo "broadcast WAV missing — building it (one-time, ~1.8 GB)..." >&2
  AUDIO_DIR="$AUDIO_DIR" OUT="$BROADCAST_WAV" "$ROOT/fixtures/make_broadcast_wav.sh"
fi

# Channel plan must match make_broadcast_wav.sh: track order = filename sort,
# stereo tracks keep L+R. Track i -> PID i.
PANS=()
MAPS=()
META=()
ch=0
i=0
for f in "$AUDIO_DIR"/*.wav; do
  chs=$(ffprobe -v error -show_entries stream=channels -of csv=p=0 "$f")
  if [ "$chs" = "1" ]; then
    PANS+=( "[s${i}]pan=stereo|c0=c${ch}|c1=c${ch}[a${i}]" )
    ch=$((ch + 1))
  else
    PANS+=( "[s${i}]pan=stereo|c0=c${ch}|c1=c$((ch + 1))[a${i}]" )
    ch=$((ch + 2))
  fi
  MAPS+=( -map "[a${i}]" )
  META+=( -metadata:"s:a:${i}" "language=ch${i}" )
  i=$((i + 1))
done

SPLITS=()
for ((k = 0; k < i; k++)); do SPLITS+=( "[s${k}]" ); done
FC="[0:a]asplit=${i}$(IFS=''; echo "${SPLITS[*]}")"
FC+=";$(IFS=';'; echo "${PANS[*]}")"

echo "PCM: ${i} tracks as ${i} stereo s302m PIDs from ${BROADCAST_WAV}" >&2
exec ffmpeg -re \
  -stream_loop -1 -i "$BROADCAST_WAV" \
  -filter_complex "$FC" \
  "${MAPS[@]}" \
  -c:a s302m -ac 2 -ar 48000 -sample_fmt s32 -strict -2 \
  "${META[@]}" \
  -f mpegts "$SRT_URL"
