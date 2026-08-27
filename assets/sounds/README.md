# Sound assets

The public source build uses the synthesised sounds in `fallback/`.

Run `generate-fallback.sh` to make these sounds again. The project distributes them under the MIT licence.

The release binary uses sound effects from the Mixkit Sound Effects Free Licence. The repository does not distribute these source files.

The ignored `encoded/` directory contains the private release copies. Run `encode-release.sh` after you download the original Mixkit WAV files.

Build the release binary with the private soundscape:

```bash
cargo build --frozen --release --features mixkit-audio
```

The feature uses fallback audio when the private files are incomplete. The release script requires every private file.

Do not commit the Mixkit WAV or OGG files.
