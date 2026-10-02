//! Builds Jolt Physics 5.6 (`third_party/jolt`) and Forge's C layer over it (`cpp/`) into one
//! static library (D-009, issue #136), with the flags Jolt's own CMake sets for
//! `CROSS_PLATFORM_DETERMINISTIC` and `DOUBLE_PRECISION`: precise floating point, no fused
//! multiply-add, the same instruction sets on every platform. Jolt is always built optimised:
//! an unoptimised build steps a hundred times slower and gains nothing in a debugger.

use std::path::{Path, PathBuf};

/// Parts of Jolt's tree left out, as its CMake leaves them out without `JPH_USE_DX12`,
/// `JPH_USE_VK`, `JPH_USE_MTL`, `JPH_USE_CPU_COMPUTE` and `JPH_OBJECT_STREAM`: the GPU compute
/// back ends, their shaders, and the object streams (serialised settings). Their base,
/// `ObjectStream/SerializableObject.cpp`, stays.
const LEFT_OUT: [&str; 7] = [
    "Compute/CPU",
    "Compute/DX12",
    "Compute/VK",
    "Compute/MTL",
    "ObjectStream/ObjectStream",
    "ObjectStream/TypeDeclarations",
    "Shaders",
];

fn sources(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("Jolt's sources (third_party/jolt)")
        .map(|e| e.expect("a directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        let relative = path
            .strip_prefix(root)
            .expect("under the root")
            .to_string_lossy()
            .replace('\\', "/");
        if LEFT_OUT.iter().any(|l| relative.starts_with(l)) {
            continue;
        }
        if path.is_dir() {
            sources(root, &path, out);
        } else if path.extension().is_some_and(|e| e == "cpp") {
            out.push(path);
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo's manifest"));
    let third_party = manifest.join("../../third_party/jolt");
    let jolt = third_party.join("Jolt");
    let mut files = Vec::new();
    sources(&jolt, &jolt, &mut files);
    files.push(manifest.join("cpp/forge_jolt.cpp"));

    let target = std::env::var("CARGO_CFG_TARGET_ARCH").expect("the target's architecture");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|e| e == "msvc");
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .include(&third_party)
        .include(manifest.join("cpp"))
        .files(&files)
        .opt_level(2)
        .debug(false)
        .warnings(false)
        .extra_warnings(false)
        .cargo_warnings(false)
        .define("JPH_CROSS_PLATFORM_DETERMINISTIC", None)
        .define("JPH_DOUBLE_PRECISION", None)
        .define("JPH_OBJECT_LAYER_BITS", "16");
    if target == "x86_64" {
        // x86-64-v3 without FMA (which cross-platform determinism forbids): what Jolt's CMake
        // picks by default.
        for define in [
            "JPH_USE_SSE4_1",
            "JPH_USE_SSE4_2",
            "JPH_USE_AVX",
            "JPH_USE_AVX2",
            "JPH_USE_LZCNT",
            "JPH_USE_TZCNT",
            "JPH_USE_F16C",
        ] {
            build.define(define, None);
        }
        if msvc {
            build.flag("/arch:AVX2");
        } else {
            for flag in [
                "-mavx2",
                "-mbmi",
                "-mpopcnt",
                "-mlzcnt",
                "-mf16c",
                "-mfpmath=sse",
            ] {
                build.flag(flag);
            }
        }
    }
    if msvc {
        for flag in ["/fp:precise", "/fp:except-", "/Zc:__cplusplus", "/GR-"] {
            build.flag(flag);
        }
    } else {
        for flag in ["-ffp-contract=off", "-fno-rtti", "-pthread"] {
            build.flag(flag);
        }
    }
    build.compile("forge_jolt");
    if !msvc {
        println!("cargo:rustc-link-lib=pthread");
    }
    println!("cargo:rerun-if-changed=cpp");
    println!("cargo:rerun-if-changed=../../third_party/jolt/Jolt");
}
