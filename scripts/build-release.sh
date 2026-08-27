#!/usr/bin/env bash

set -euo pipefail

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_dir"

package_id=$(cargo pkgid)
version=${package_id##*@}
archive_name="gibson-ui-${version}-x86_64"
stage_dir="$repo_dir/dist/$archive_name"

private_sounds=(
  suspense.ogg
  ui-sweep.ogg
  flight-short.ogg
  flight-medium-a.ogg
  flight-medium-b.ogg
  flight-long.ogg
)
for sound in "${private_sounds[@]}"; do
  if [[ ! -f "assets/sounds/encoded/$sound" ]]; then
    echo "Private Mixkit audio is incomplete. Read assets/sounds/README.md." >&2
    exit 1
  fi
done

command -v cargo-about >/dev/null || {
  echo "Install cargo-about before you build a release archive." >&2
  exit 1
}

cargo test --frozen --all-features
cargo clippy --frozen --all-targets --all-features -- -D warnings
cargo build --frozen --release --features mixkit-audio
cargo about generate --locked about.hbs | sed -e '${/^$/d;}' > THIRD_PARTY_LICENSES.html

rm -rf -- "$stage_dir"
mkdir -p -- "$stage_dir" "$repo_dir/dist"
install -Dm755 target/release/gibson-ui "$stage_dir/gibson-ui"
install -Dm644 packaging/gibson-ui.desktop "$stage_dir/gibson-ui.desktop"
install -Dm644 assets/images/gibson-ui.png "$stage_dir/gibson-ui.png"
install -Dm644 docs/gibson-ui.1 "$stage_dir/gibson-ui.1"
install -Dm644 LICENSE "$stage_dir/LICENSE"
install -Dm644 THIRD_PARTY.md "$stage_dir/THIRD_PARTY.md"
install -Dm644 THIRD_PARTY_LICENSES.html "$stage_dir/THIRD_PARTY_LICENSES.html"

tar --zstd --create --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
  --file "$repo_dir/dist/$archive_name.tar.zst" \
  --directory "$repo_dir/dist" "$archive_name"

cd "$repo_dir/dist"
sha256sum "$archive_name.tar.zst" > "$archive_name.tar.zst.sha256"
cat "$archive_name.tar.zst.sha256"
