#!/usr/bin/env bash
set -euo pipefail

sound_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$sound_dir/encoded"

ffmpeg -y -v error -i "$sound_dir/mixkit-synth-suspense-music-681.wav" \
  -af 'atrim=start=0.10:end=212.93,asetpts=PTS-STARTPTS,afade=t=in:st=0:d=0.03,afade=t=out:st=212.55:d=0.28' \
  -c:a libvorbis -q:a 4 "$sound_dir/encoded/suspense.ogg"

ffmpeg -y -v error -i "$sound_dir/mixkit-sci-fi-sweep-2522.wav" \
  -af 'atrim=start=0.105:end=0.90,asetpts=PTS-STARTPTS,afade=t=in:st=0:d=0.005,afade=t=out:st=0.69:d=0.10' \
  -c:a libvorbis -q:a 5 "$sound_dir/encoded/ui-sweep.ogg"

ffmpeg -y -v error -i "$sound_dir/mixkit-flying-fast-swoosh-1469.wav" \
  -af 'atrim=start=0.18:end=1.16,asetpts=PTS-STARTPTS,afade=t=in:st=0:d=0.01,afade=t=out:st=0.88:d=0.10' \
  -c:a libvorbis -q:a 5 "$sound_dir/encoded/flight-short.ogg"

ffmpeg -y -v error -i "$sound_dir/mixkit-fast-sweeping-transition-164.wav" \
  -af 'atrim=start=0:end=1.72,asetpts=PTS-STARTPTS,afade=t=out:st=1.60:d=0.12' \
  -c:a libvorbis -q:a 5 "$sound_dir/encoded/flight-medium-a.ogg"

ffmpeg -y -v error -i "$sound_dir/mixkit-transition-flying-swoosh-3161.wav" \
  -af 'atrim=start=0.16:end=2.10,asetpts=PTS-STARTPTS,afade=t=in:st=0:d=0.01,afade=t=out:st=1.80:d=0.14' \
  -c:a libvorbis -q:a 5 "$sound_dir/encoded/flight-medium-b.ogg"

ffmpeg -y -v error -i "$sound_dir/mixkit-sci-fi-rocket-engine-1723.wav" \
  -af 'atrim=start=0.13:end=7.15,asetpts=PTS-STARTPTS,afade=t=in:st=0:d=0.05,afade=t=out:st=6.70:d=0.30' \
  -c:a libvorbis -q:a 5 "$sound_dir/encoded/flight-long.ogg"
