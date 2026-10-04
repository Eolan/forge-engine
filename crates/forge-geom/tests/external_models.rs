//! The labs' external models (D-048), when `tools/fetch-assets.sh` has fetched them into
//! `assets/external/`: every one loads. Without them (CI) there is nothing to check.

use forge_geom::model::load_gltf;

/// The labs' external models (D-048), when `tools/fetch-assets.sh` has fetched them: each
/// loads with meshes, its images and what Forge does not draw yet listed.
#[test]
fn the_external_models_load() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/external");
    let Ok(models) = std::fs::read_dir(&root) else {
        eprintln!("no external models (tools/fetch-assets.sh fetches them)");
        return;
    };
    for model in models.flatten() {
        for kind in ["glTF-Binary", "glTF"] {
            let Ok(files) = std::fs::read_dir(model.path().join(kind)) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if !matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("glb" | "gltf")
                ) {
                    continue;
                }
                let m = load_gltf(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                let triangles: usize = m.meshes.iter().map(|m| m.mesh.triangle_count()).sum();
                assert!(triangles > 0, "{}", path.display());
                eprintln!(
                    "{}: {} meshes, {triangles} triangles, {} images, unsupported: {:?}",
                    path.file_name().unwrap().to_string_lossy(),
                    m.meshes.len(),
                    m.images.len(),
                    m.unsupported
                );
            }
        }
    }
}
