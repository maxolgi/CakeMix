#!/bin/bash
# One-time offline conversion: merges the session tracks into a single
# multi-channel "broadcast" WAV — channel order = track order (filename sort),
# stereo tracks keep L+R — resampled once to 48 kHz, all tracks sample-aligned
# on one clock with one shared loop point. This is what makes the live stream
# phase-coherent: one decode, one resample, no per-track drift.
#
# Output: tmp/broadcast30.wav (~1.8 GB, gitignored). Rebuilt when any source
# track is newer than the output.
#
# Usage: ./fixtures/make_broadcast_wav.sh
# Override source dir: AUDIO_DIR=/path/to/wavs ./fixtures/make_broadcast_wav.sh

set -euo pipefail

AUDIO_DIR="${AUDIO_DIR:-$HOME/Downloads/audio/PawelMaciwoda_1OfTheFirst_Full}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${OUT:-$ROOT/tmp/broadcast30.wav}"

if [ ! -d "$AUDIO_DIR" ]; then
  echo "error: AUDIO_DIR does not exist: ${AUDIO_DIR}" >&2
  exit 1
fi

if [ -f "$OUT" ] && [ -z "$(find "$AUDIO_DIR" -name '*.wav' -newer "$OUT" -print -quit)" ]; then
  echo "up to date: $OUT" >&2
  exit 0
fi

FFMPEG_INPUTS=()
FILTERS=()
PINS=()
MAX_DUR=0
ch=0
i=0
for f in "$AUDIO_DIR"/*.wav; do
  FFMPEG_INPUTS+=( -i "$f" )
  chs=$(ffprobe -v error -show_entries stream=channels -of csv=p=0 "$f")
  dur=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$f")
  MAX_DUR=$(awk -v a="$dur" -v b="$MAX_DUR" 'BEGIN{print (a > b) ? a : b}')
  if [ "$chs" = "1" ]; then
    FILTERS+=( "[${i}:a]apad[m${ch}]" )
    PINS+=( "[m${ch}]" )
    ch=$((ch + 1))
  else
    FILTERS+=( "[${i}:a]apad,asplit=2[s${i}a][s${i}b]" )
    FILTERS+=( "[s${i}a]channelmap=map=0[m${ch}]" )
    FILTERS+=( "[s${i}b]channelmap=map=1[m$((ch + 1))]" )
    PINS+=( "[m${ch}]" "[m$((ch + 1))]" )
    ch=$((ch + 2))
  fi
  i=$((i + 1))
done

FC="$(IFS=';'; echo "${FILTERS[*]}")"
FC+=";$(IFS=''; echo "${PINS[*]}")"
FC+="amerge=inputs=${ch}[aout]"

echo "building $OUT: ${i} tracks -> ${ch} channels, ${MAX_DUR}s @ 48 kHz" >&2
mkdir -p "$(dirname "$OUT")"
ffmpeg -hide_banner -y \
  "${FFMPEG_INPUTS[@]}" \
  -filter_complex "$FC" \
  -map "[aout]" -ar 48000 -t "$MAX_DUR" -c:a pcm_s32le \
  "$OUT" 2>&1 | tail -2
echo "done: $OUT" >&2
