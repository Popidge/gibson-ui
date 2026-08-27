#!/usr/bin/env bash
set -euo pipefail

sound_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
output_dir="$sound_dir/fallback"
mkdir -p "$output_dir"

encode() {
  local expression="$1"
  local duration="$2"
  local output="$3"
  ffmpeg -y -v error \
    -f lavfi -i "aevalsrc=${expression}:s=48000:d=${duration}" \
    -c:a libvorbis -q:a 3 "$output_dir/$output"
}

encode \
  '0.10*sin(2*PI*55*t)+0.055*sin(2*PI*82.5*t)+0.025*sin(2*PI*(110+3*sin(2*PI*0.125*t))*t)' \
  24 \
  suspense.ogg

encode \
  '0.22*sin(2*PI*(420*t+950*t*t))*exp(-4.2*t)' \
  0.72 \
  ui-sweep.ogg

encode \
  '0.24*sin(2*PI*(105*t+520*t*t))*sin(PI*t/1.0)' \
  1.0 \
  flight-short.ogg

encode \
  '0.20*sin(2*PI*(72*t+260*t*t))*sin(PI*t/1.8)' \
  1.8 \
  flight-medium-a.ogg

encode \
  '0.18*sin(2*PI*(390*t-70*t*t))*sin(PI*t/2.1)+0.05*sin(2*PI*95*t)' \
  2.1 \
  flight-medium-b.ogg

encode \
  '0.14*sin(2*PI*64*t)+0.07*sin(2*PI*(96+8*sin(2*PI*t/6))*t)' \
  6.0 \
  flight-long.ogg
