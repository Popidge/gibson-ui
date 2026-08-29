#!/usr/bin/env bash

set -euo pipefail

repository_url="https://github.com/Popidge/gibson-ui"
install_prefix=${GIBSON_INSTALL_PREFIX:-}
requested_version=${GIBSON_VERSION:-}

fail() {
  printf 'GIBSON install: %s\n' "$*" >&2
  exit 1
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    fail "Required command not found: $1"
  fi
}

if [[ $(uname -s) != "Linux" ]]; then
  fail "GIBSON release binaries support Linux only."
fi

case $(uname -m) in
  x86_64 | amd64) architecture="x86_64" ;;
  *) fail "GIBSON release binaries support x86_64 only." ;;
esac

for required_command in curl install mktemp sha256sum; do
  require_command "$required_command"
done

if command -v bsdtar >/dev/null 2>&1; then
  extractor="bsdtar"
elif command -v tar >/dev/null 2>&1 && command -v zstd >/dev/null 2>&1; then
  extractor="tar"
else
  fail "Install libarchive or GNU tar with zstd support."
fi

if [[ -z $install_prefix ]]; then
  [[ -n ${HOME:-} ]] || fail "HOME is not set."
  install_prefix="$HOME/.local"
fi
[[ $install_prefix == /* ]] || fail "GIBSON_INSTALL_PREFIX must be an absolute path."

curl_options=(
  --fail
  --location
  --proto '=https'
  --retry 3
  --show-error
  --silent
  --tlsv1.2
)

if [[ -n $requested_version ]]; then
  version=${requested_version#v}
  tag="v$version"
else
  latest_url=$(curl "${curl_options[@]}" --output /dev/null --write-out '%{url_effective}' \
    "$repository_url/releases/latest")
  tag=${latest_url##*/}
  version=${tag#v}
fi

if [[ ! $tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  fail "Release version is not valid: $tag"
fi

archive_name="gibson-ui-$version-$architecture.tar.zst"
checksum_name="$archive_name.sha256"
release_url="$repository_url/releases/download/$tag"
temporary_root=${TMPDIR:-/tmp}
work_dir=$(mktemp -d "$temporary_root/gibson-install.XXXXXXXX")

cleanup() {
  if [[ -d ${work_dir:-} ]]; then
    rm -r -- "$work_dir"
  fi
}
trap cleanup EXIT

printf 'Downloading GIBSON %s...\n' "$version"
curl "${curl_options[@]}" --output "$work_dir/$archive_name" \
  "$release_url/$archive_name"
curl "${curl_options[@]}" --output "$work_dir/$checksum_name" \
  "$release_url/$checksum_name"

IFS=' ' read -r expected_hash _ < "$work_dir/$checksum_name"
actual_hash_line=$(sha256sum "$work_dir/$archive_name")
actual_hash=${actual_hash_line%% *}
if [[ ! $expected_hash =~ ^[0-9a-f]{64}$ || $actual_hash != "$expected_hash" ]]; then
  fail "The release archive checksum does not match."
fi

if [[ $extractor == "bsdtar" ]]; then
  bsdtar -xf "$work_dir/$archive_name" -C "$work_dir"
else
  tar --zstd -xf "$work_dir/$archive_name" -C "$work_dir"
fi

release_dir="$work_dir/gibson-ui-$version-$architecture"
release_files=(
  gibson-ui
  gibson-ui.desktop
  gibson-ui.png
  gibson-ui.1
  LICENSE
  THIRD_PARTY.md
  THIRD_PARTY_LICENSES.html
)
for release_file in "${release_files[@]}"; do
  [[ -f "$release_dir/$release_file" ]] || fail "Release file not found: $release_file"
done

install -Dm755 "$release_dir/gibson-ui" "$install_prefix/bin/gibson-ui"
install -Dm644 "$release_dir/gibson-ui.desktop" \
  "$install_prefix/share/applications/gibson-ui.desktop"
install -Dm644 "$release_dir/gibson-ui.png" "$install_prefix/share/pixmaps/gibson-ui.png"
install -Dm644 "$release_dir/gibson-ui.1" "$install_prefix/share/man/man1/gibson-ui.1"
install -Dm644 "$release_dir/LICENSE" "$install_prefix/share/licenses/gibson-ui/LICENSE"
install -Dm644 "$release_dir/THIRD_PARTY.md" \
  "$install_prefix/share/doc/gibson-ui/THIRD_PARTY.md"
install -Dm644 "$release_dir/THIRD_PARTY_LICENSES.html" \
  "$install_prefix/share/doc/gibson-ui/THIRD_PARTY_LICENSES.html"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$install_prefix/share/applications" >/dev/null 2>&1 || true
fi

printf 'Installed GIBSON %s in %s.\n' "$version" "$install_prefix"
printf 'Run: %s/bin/gibson-ui --cockpit\n' "$install_prefix"
case ":${PATH:-}:" in
  *":$install_prefix/bin:"*) ;;
  *) printf 'Add %s/bin to PATH before you run gibson-ui by name.\n' "$install_prefix" ;;
esac
