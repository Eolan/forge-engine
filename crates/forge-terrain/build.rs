//! The code digests of the island's derived products (#208, D-053): for each product, a digest
//! of the source files that make it, so a code change remakes it without a version bumped by
//! hand. A list too broad only costs a remake; too narrow serves a stale product, so each
//! names the whole tangle it calls into (`river`, `channel` and `lake` call one another).
//! Upstream products' changes arrive through their keys, which their consumers' keys include.

use std::fmt::Write as _;
use std::path::Path;

/// The procgen modules every water product calls into.
const RIVERS: &[&str] = &[
    "river",
    "channel",
    "lake",
    "field",
    "flow",
    "hydrology",
    "noise",
];

/// Each product: its constant's name, `forge-procgen` modules, whether [`RIVERS`] too, and
/// other files (relative to the workspace).
const PRODUCTS: &[(&str, &[&str], bool, &[&str])] = &[
    ("ERODED", &["island", "erosion"], true, &[]),
    (
        "HEIGHTS",
        &["coast", "valley"],
        true,
        &["crates/forge-terrain/src/heights.rs"],
    ),
    (
        "WATER",
        &[],
        true,
        &[
            "crates/forge-terrain/src/water.rs",
            "crates/forge-terrain/src/heights.rs",
        ],
    ),
    (
        "LAYERS",
        &["layers", "beach", "sites", "valley", "island", "coast"],
        true,
        &[
            "crates/forge-terrain/src/layers.rs",
            "crates/forge-terrain/src/heights.rs",
        ],
    ),
    (
        "AMPLIFIED",
        &["amplify", "field", "flow", "hydrology", "noise"],
        false,
        &["crates/forge-terrain/src/drawn.rs"],
    ),
    (
        "DRAWN",
        &["coast"],
        true,
        &[
            "crates/forge-terrain/src/drawn.rs",
            "crates/forge-geom/src/city.rs",
        ],
    ),
    ("STONES", &[], true, &["crates/forge-terrain/src/stones.rs"]),
    // The island's ground tiles as `forge-geom` cooks them.
    (
        "TILES",
        &[],
        false,
        &[
            "crates/forge-terrain/src/tiles.rs",
            "crates/forge-geom/src/cache.rs",
            "crates/forge-geom/src/city.rs",
            "crates/forge-geom/src/lod.rs",
            "crates/forge-geom/src/meshlet.rs",
            "crates/forge-geom/src/page.rs",
            "crates/forge-geom/src/procedural.rs",
            "crates/forge-geom/src/skin.rs",
            "crates/forge-geom/src/stone.rs",
        ],
    ),
];

/// Files in every list: the cache's own encoding and the deterministic foundations.
const ALWAYS: &[&str] = &[
    "crates/forge-core/src/derived.rs",
    "crates/forge-core/src/seed.rs",
    "crates/forge-core/src/hash.rs",
    "crates/forge-core/src/dmath.rs",
    "crates/forge-terrain/build.rs",
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
    let mut out =
        String::from("// Written by build.rs: the island's products' code digests (#208).\n");
    for (name, modules, rivers, files) in PRODUCTS {
        let mut paths: Vec<String> = modules
            .iter()
            .chain(if *rivers { RIVERS } else { &[] })
            .map(|m| format!("crates/forge-procgen/src/{m}.rs"))
            .chain(files.iter().chain(ALWAYS).map(|f| (*f).to_owned()))
            .collect();
        paths.sort();
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
            "/// The code digest of the island's {} product.",
            name.to_lowercase()
        )
        .unwrap();
        writeln!(out, "pub(crate) const CODE_{name}: u64 = 0x{hash:016x};").unwrap();
    }
    let dest = Path::new(&std::env::var("OUT_DIR").expect("cargo sets it")).join("code_digests.rs");
    std::fs::write(dest, out).expect("the code digests");
}
