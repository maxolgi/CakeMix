#!/bin/bash
# Real-audio source, 16-channel cut — pushes the FIRST 16 channels of the
# same broadcast WAV (tmp/broadcast30.wav) over SRT to the running WebSRT
# gateway. Same rules as stream_pcm_real.sh: one stereo s302m PID per source
# track, mono tracks duplicated L=R (s302m has no mono layout), 48 kHz,
# looped forever on a single shared loop point. Audio-only MPEG-2 TS, no
# video. With the current session dir the first 16 channels are exactly
# tracks 01-14 (kick/snare/hat mics, overheads, room, toms, ac+el guitars).
#
# Usage: ./fixtures/stream_pcm_real16.sh
# Override source dir:  AUDIO_DIR=/path/to/wavs ./fixtures/stream_pcm_real16.sh
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

# Channel plan must match make_broadcast_wav.sh: track order = filename sort,
# stereo tracks keep L+R. Track i -> PID i. Stop once MAX_CH channels of the
# WAV are consumed; a stereo track that would straddle the cap ends the list.
PANS=()
MAPS=()
META=()
ch=0
i=0
for f in "$AUDIO_DIR"/*.wav; do
  if [ "$ch" -ge "$MAX_CH" ]; then
    break
  fi
  chs=$(ffprobe -v error -show_entries stream=channels -of csv=p=0 "$f")
  if [ "$chs" = "1" ]; then
    PANS+=( "[s${i}]pan=stereo|c0=c${ch}|c1=c${ch}[a${i}]" )
    ch=$((ch + 1))
  elif [ $((ch + 2)) -le "$MAX_CH" ]; then
    PANS+=( "[s${i}]pan=stereo|c0=c${ch}|c1=c$((ch + 1))[a${i}]" )
    ch=$((ch + 2))
  else
    break
  fi
  MAPS+=( -map "[a${i}]" )
  META+=( -metadata:"s:a:${i}" "language=ch${i}" )
  i=$((i + 1))
done

if [ "$ch" -ne "$MAX_CH" ]; then
  echo "error: channel plan covered only ${ch} of ${MAX_CH} channels — AUDIO_DIR changed?" >&2
  exit 1
fi

SPLITS=()
for ((k = 0; k < i; k++)); do SPLITS+=( "[s${k}]" ); done
FC="[0:a]asplit=${i}$(IFS=''; echo "${SPLITS[*]}")"
FC+=";$(IFS=';'; echo "${PANS[*]}")"

echo "PCM: first ${MAX_CH} channels as ${i} stereo s302m PIDs from ${BROADCAST_WAV}" >&2
exec ffmpeg -re \
  -stream_loop -1 -i "$BROADCAST_WAV" \
  -filter_complex "$FC" \
  "${MAPS[@]}" \
  -c:a s302m -ac 2 -ar 48000 -sample_fmt s32 -strict -2 \
  "${META[@]}" \
  -f mpegts "$SRT_URL"
