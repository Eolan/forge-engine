//! The code digests of city-plan's derived products (#345, D-053): for each product, a digest of
//! the source files that make it, so a code change remakes it without a version bumped by hand. A
//! list too broad only costs a remake; too narrow serves a stale product.

use std::fmt::Write as _;
use std::path::Path;

/// Each product: its constant's name and the files that make it (relative to the workspace).
const PRODUCTS: &[(&str, &[&str])] = &[
    // The city's blocks' ground: the street ground's cuts and their triangulation, the polygon
    // library's version (the lock file), and the step itself with its levels and sections.
    (
        "BLOCKS_GROUND",
        &[
            "crates/forge-procgen/src/city/ground.rs",
            "crates/forge-procgen/src/city/geom.rs",
            "Cargo.lock",
            "demos/city-plan/src/lib.rs",
        ],
    ),
];

/// Files in every list: the cache's own encoding and this script.
const ALWAYS: &[&str] = &[
    "crates/forge-core/src/derived.rs",
    "crates/forge-core/src/pack.rs",
    "demos/city-plan/build.rs",
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
    let mut out = String::from("// Written by build.rs: city-plan's products' code digests.\n");
    for (name, files) in PRODUCTS {
        let mut paths: Vec<&str> = files.iter().chain(ALWAYS).copied().collect();
        paths.sort_unstable();
        paths.dedup();
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for path in &paths {
            let file = root.join(path);
            println!("cargo:rerun-if-changed={}", file.display());
            let bytes = std::fs::read(&file).unwrap_or_else(|e| panic!("{path}: {e}"));
            digest(&mut hash, path.as_bytes());
            digest(&mut hash, &bytes);
        }
        writeln!(
            out,
            "/// The code digest of the {} product.",
            name.to_lowercase().replace('_', " ")
        )
        .unwrap();
        writeln!(out, "pub(crate) const CODE_{name}: u64 = 0x{hash:016x};").unwrap();
    }
    let dest = Path::new(&std::env::var("OUT_DIR").expect("cargo sets it")).join("code_digests.rs");
    std::fs::write(dest, out).expect("the code digests");
}
