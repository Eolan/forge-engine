//! With the `steam-audio` feature on Windows: copies Steam Audio's `phonon.dll` from
//! `STEAMAUDIO_LIB_DIR` (where `audionimbus-sys` links its import library) into this build's
//! output directory and adds that directory to the link search path. Cargo puts search paths
//! inside the target directory on `PATH` for `cargo run` and `cargo test`, so the tests and
//! the examples find the library without anyone editing `PATH`. Without the SDK it does
//! nothing: the feature still type-checks (Clippy on CI) but cannot link.

use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-env-changed=STEAMAUDIO_LIB_DIR");
    println!("cargo::rerun-if-changed=build.rs");
    let windows = std::env::var("CARGO_CFG_WINDOWS").is_ok();
    if std::env::var("CARGO_FEATURE_STEAM_AUDIO").is_err() || !windows {
        return;
    }
    let Ok(dir) = std::env::var("STEAMAUDIO_LIB_DIR") else {
        return;
    };
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let mut copied = false;
    for name in ["phonon.dll", "TrueAudioNext.dll", "GPUUtilities.dll"] {
        let from = PathBuf::from(&dir).join(name);
        if from.is_file() {
            println!("cargo::rerun-if-changed={}", from.display());
            copied |= std::fs::copy(&from, out.join(name)).is_ok();
        }
    }
    if copied {
        println!("cargo::rustc-link-search=native={}", out.display());
    }
}
