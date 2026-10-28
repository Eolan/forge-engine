//! The cooked meshes' cache key holds `forge_geom::cache::COOK_VERSION`, not meshoptimizer's
//! version: a new `meshopt` in Cargo.lock (a weekly batch's patch release too) must come with a
//! bump, or the caches serve meshes the old one cooked (the review of 2026-10-28, #361). Here, not
//! in `cache.rs`, whose text is part of the cook code's digest.

use forge_geom::cache::COOK_VERSION;

/// The `meshopt` release and the cook version set for it: move both together.
const COOKED_WITH: (&str, u32) = ("0.7.0", 6);

#[test]
fn the_cook_version_follows_meshopt() {
    let lock = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"),
    )
    .expect("the workspace's Cargo.lock");
    let version = lock
        .split("[[package]]")
        .find(|p| p.contains("\nname = \"meshopt\"\n"))
        .and_then(|p| p.lines().find_map(|l| l.strip_prefix("version = \"")))
        .and_then(|v| v.strip_suffix('"'))
        .expect("meshopt in Cargo.lock");
    assert_eq!(
        (version, COOK_VERSION),
        COOKED_WITH,
        "meshopt {version} with COOK_VERSION {COOK_VERSION}: bump COOK_VERSION for the new \
         meshopt, then COOKED_WITH"
    );
}
