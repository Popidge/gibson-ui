use std::path::Path;

const PRIVATE_SOUNDS: [&str; 6] = [
    "suspense.ogg",
    "ui-sweep.ogg",
    "flight-short.ogg",
    "flight-medium-a.ogg",
    "flight-medium-b.ogg",
    "flight-long.ogg",
];

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_mixkit_audio)");
    println!("cargo::rerun-if-env-changed=CARGO_FEATURE_MIXKIT_AUDIO");

    let sound_dir = Path::new("assets/sounds/encoded");
    for name in PRIVATE_SOUNDS {
        println!("cargo::rerun-if-changed={}", sound_dir.join(name).display());
    }

    if std::env::var_os("CARGO_FEATURE_MIXKIT_AUDIO").is_none() {
        return;
    }

    if PRIVATE_SOUNDS
        .iter()
        .all(|name| sound_dir.join(name).is_file())
    {
        println!("cargo::rustc-cfg=has_mixkit_audio");
    } else {
        println!(
            "cargo::warning=private Mixkit audio is incomplete; using the public fallback soundscape"
        );
    }
}
