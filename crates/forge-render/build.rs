//! The code digest of the impostors' bake (D-075 step 4, kept in D-053's derived cache): a digest
//! of the source files that make an impostor's atlases, so a code change bakes them again without
//! a version bumped by hand. A list too broad only costs a bake; too narrow serves a stale one.

use std::path::Path;

/// The files that make an impostor's atlases (relative to the workspace): the bake and its
/// shaders, the resolve that draws its frames, the material table and the textures it reads, and
/// the cache's own encoding.
const IMPOSTOR_BAKE: &[&str] = &[
    "crates/forge-core/src/derived.rs",
    "crates/forge-core/src/pack.rs",
    "crates/forge-render/build.rs",
    "crates/forge-render/src/impostor.rs",
    "crates/forge-render/src/material.rs",
    "crates/forge-render/src/meshlet.rs",
    "crates/forge-render/src/textures.rs",
    "shaders/bindless.slang",
    "shaders/impostor.slang",
    "shaders/impostor_layout.slang",
    "shaders/meshlet.slang",
    "shaders/meshlet_bake.slang",
];

/// FNV-1a, 64 bits, over the text with its line endings made `\n` (a checkout's CRLF must not
/// change the digest).
fn digest(hash: &mut u64, bytes: &[u8]) {
    for &b in bytes.iter().filter(|&&b| b != b'\r') {
        *hash ^= u64::from(b);
        *hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it");
    let root = Path::new(&manifest).join("../..");
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for path in IMPOSTOR_BAKE {
        let file = root.join(path);
        println!("cargo:rerun-if-changed={}", file.display());
        let bytes = std::fs::read(&file).unwrap_or_else(|e| panic!("{path}: {e}"));
        digest(&mut hash, path.as_bytes());
        digest(&mut hash, &bytes);
    }
    let out = format!(
        "// Written by build.rs.\n/// The code digest of the impostors' bake.\npub(crate) const CODE_IMPOSTOR_BAKE: u64 = 0x{hash:016x};\n"
    );
    let dest = Path::new(&std::env::var("OUT_DIR").expect("cargo sets it")).join("code_digests.rs");
    std::fs::write(dest, out).expect("the code digests");
}
