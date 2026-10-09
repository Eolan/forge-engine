use std::collections::{BTreeMap, HashMap};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ash::vk;
use xxhash_rust::xxh3::Xxh3;

use crate::device::Device;
use crate::error::{GpuError, Result};

/// Shader stage, mapped to a `slangc -stage` name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderStage {
    /// Task (amplification) shader.
    Task,
    /// Mesh shader.
    Mesh,
    /// Vertex shader.
    Vertex,
    /// Fragment shader.
    Fragment,
    /// Compute shader.
    Compute,
}

impl ShaderStage {
    fn slang_name(self) -> &'static str {
        match self {
            Self::Task => "amplification",
            Self::Mesh => "mesh",
            Self::Vertex => "vertex",
            Self::Fragment => "fragment",
            Self::Compute => "compute",
        }
    }

    fn from_slang_name(name: &str) -> Option<Self> {
        [
            Self::Task,
            Self::Mesh,
            Self::Vertex,
            Self::Fragment,
            Self::Compute,
        ]
        .into_iter()
        .find(|stage| stage.slang_name() == name)
    }

    /// The Vulkan stage flag.
    pub fn vk(self) -> vk::ShaderStageFlags {
        match self {
            Self::Task => vk::ShaderStageFlags::TASK_EXT,
            Self::Mesh => vk::ShaderStageFlags::MESH_EXT,
            Self::Vertex => vk::ShaderStageFlags::VERTEX,
            Self::Fragment => vk::ShaderStageFlags::FRAGMENT,
            Self::Compute => vk::ShaderStageFlags::COMPUTE,
        }
    }
}

/// An entry list as text: one `file entry stage` line each.
fn format_entries(entries: &[ShaderEntry]) -> String {
    entries
        .iter()
        .map(|e| format!("{} {} {}\n", e.file, e.entry, e.stage.slang_name()))
        .collect()
}

/// An entry list's text read back, skipping the lines it cannot read.
fn parse_entries(text: &str) -> Vec<ShaderEntry> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let (file, entry, stage) = (words.next()?, words.next()?, words.next()?);
            Some(ShaderEntry {
                file: file.to_owned(),
                entry: entry.to_owned(),
                stage: ShaderStage::from_slang_name(stage)?,
            })
        })
        .collect()
}

/// One entry point a program compiles: what [`ShaderCompiler::warm`] compiles ahead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderEntry {
    /// The file, relative to the shader directory.
    pub file: String,
    /// The entry point.
    pub entry: String,
    /// Its stage.
    pub stage: ShaderStage,
}

/// The files a shader's text imports or includes, relative to the shader directory: `import
/// a.b_c;` is `a/b_c.slang` (or `a/b-c.slang`, which Slang also tries), `#include "x"` and
/// `__include "x"` are `x` beside the importing file. Over-matching (a line inside a block
/// comment) only costs a needless recompile.
fn imports(shader_dir: &Path, importer: &str, text: &str) -> Vec<String> {
    let beside = Path::new(importer)
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|p| !p.is_empty());
    let resolve = |name: String| -> String {
        let local = beside
            .as_ref()
            .map(|dir| format!("{dir}/{name}"))
            .filter(|path| shader_dir.join(path).exists());
        local.unwrap_or(name)
    };
    let mut found = Vec::new();
    for line in text.lines().map(str::trim_start) {
        if let Some(rest) = line.strip_prefix("import ") {
            let module = rest.split(';').next().unwrap_or_default().trim();
            let path = module.replace('.', "/");
            let underscored = resolve(format!("{path}.slang"));
            let hyphenated = resolve(format!("{}.slang", path.replace('_', "-")));
            if !shader_dir.join(&underscored).exists() && shader_dir.join(&hyphenated).exists() {
                found.push(hyphenated);
            } else {
                found.push(underscored);
            }
        } else if let Some(rest) = line
            .strip_prefix("#include ")
            .or_else(|| line.strip_prefix("__include "))
            && let Some(name) = rest.split('"').nth(1)
        {
            found.push(resolve(name.to_owned()));
        }
    }
    found
}

/// Compiles Slang to SPIR-V with `slangc`, caching each entry under a hash of its file and the
/// files it imports (#209).
///
/// A clone, for another thread, shares the list of entries asked for: the finishing step asks
/// for its shaders on a worker (#201), and the list saved for the next start must have them.
#[derive(Clone)]
pub struct ShaderCompiler {
    slangc: PathBuf,
    shader_dir: PathBuf,
    cache_dir: PathBuf,
    optimize: bool,
    version: String,
    /// Every entry asked for so far, in order (issue #25), shared with the clones.
    requested: Arc<Mutex<Vec<ShaderEntry>>>,
}

impl ShaderCompiler {
    /// Finds `slangc` (`FORGE_SLANGC`, then `$VULKAN_SDK/Bin`, then `PATH`).
    pub fn new(
        shader_dir: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
        optimize: bool,
    ) -> Result<Self> {
        let exe = if cfg!(windows) {
            "slangc.exe"
        } else {
            "slangc"
        };
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(p) = env::var_os("FORGE_SLANGC") {
            candidates.push(PathBuf::from(p));
        }
        if let Some(sdk) = env::var_os("VULKAN_SDK") {
            candidates.push(Path::new(&sdk).join("Bin").join(exe));
            candidates.push(Path::new(&sdk).join("bin").join(exe));
        }
        candidates.push(PathBuf::from(exe));
        let mut found = None;
        for candidate in candidates {
            if let Ok(output) = Command::new(&candidate).arg("-v").output() {
                let version = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                let version = if version.is_empty() {
                    String::from_utf8_lossy(&output.stdout).trim().to_owned()
                } else {
                    version
                };
                found = Some((candidate, version));
                break;
            }
        }
        let (slangc, version) = found.ok_or_else(|| {
            GpuError::Shader("slangc not found: install the Vulkan SDK or set FORGE_SLANGC".into())
        })?;
        let cache_dir = cache_dir.into();
        fs::create_dir_all(&cache_dir)?;
        tracing::info!(slangc = %slangc.display(), %version, "shader compiler");
        // The SPIR-V cached before #209 (`file@entry@hash.spv`, one set per edit of any shader)
        // is never read again: removed beside the start, once.
        let sweep = cache_dir.clone();
        let _ = std::thread::Builder::new()
            .name("shader cache sweep".to_owned())
            .spawn(move || {
                let Ok(dir) = fs::read_dir(&sweep) else {
                    return;
                };
                let mut removed = 0usize;
                for path in dir.filter_map(|e| e.ok().map(|e| e.path())) {
                    let old = path.extension() == Some(OsStr::new("spv"))
                        && path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .is_some_and(|s| s.split('@').count() == 3);
                    if old && fs::remove_file(&path).is_ok() {
                        removed += 1;
                    }
                }
                if removed > 0 {
                    tracing::info!(removed, "shader cache: the SPIR-V of the old keys removed");
                }
            });
        Ok(Self {
            slangc,
            shader_dir: shader_dir.into(),
            cache_dir,
            optimize,
            version,
            requested: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// The cache directory (the compiled SPIR-V, and the entry lists of [`Self::save_entries`]).
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Every entry asked of [`Self::compile`] so far, in order, without repeats.
    pub fn requested(&self) -> Vec<ShaderEntry> {
        self.requested
            .lock()
            .map(|list| list.clone())
            .unwrap_or_default()
    }

    /// Writes the entries asked for so far to `path`, one `file entry stage` line each: the
    /// list [`Self::warm`] compiles ahead at the next start (issue #25).
    pub fn save_entries(&self, path: &Path) -> Result<()> {
        fs::write(path, format_entries(&self.requested()))?;
        Ok(())
    }

    /// The entries [`Self::save_entries`] wrote to `path` (none if it is missing or unreadable).
    pub fn load_entries(path: &Path) -> Vec<ShaderEntry> {
        parse_entries(&fs::read_to_string(path).unwrap_or_default())
    }

    /// Compiles `entries` into the cache on `threads` threads, so that the program's own
    /// requests find them there: after a shader change, a loading screen compiles them while
    /// it shows (issue #25). Returns how many were compiled, not found in the cache.
    pub fn warm(&self, entries: &[ShaderEntry], threads: usize) -> Result<usize> {
        self.warm_counted(entries, threads, &WarmProgress::default())
    }

    /// [`Self::warm`], counting into `progress` how many entries the cache lacks and how many
    /// of those are compiled (#200: the loading screen's bar).
    pub fn warm_counted(
        &self,
        entries: &[ShaderEntry],
        threads: usize,
        progress: &WarmProgress,
    ) -> Result<usize> {
        let mut hashes = HashMap::new();
        let mut missing: Vec<&ShaderEntry> = Vec::new();
        for e in entries {
            let hash = match hashes.get(&e.file) {
                Some(&hash) => hash,
                None => {
                    // An entry list can name a file since removed: skipped, not an error.
                    let Ok(hash) = self.source_hash(&e.file) else {
                        continue;
                    };
                    *hashes.entry(e.file.clone()).or_insert(hash)
                }
            };
            if !self.cached_path(&e.file, &e.entry, hash).exists() {
                missing.push(e);
            }
        }
        progress.missing.store(missing.len(), Ordering::Relaxed);
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..threads.max(1))
                .map(|_| {
                    scope.spawn(|| -> Result<()> {
                        loop {
                            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some(e) = missing.get(i) else {
                                return Ok(());
                            };
                            // Not listed as asked for: an entry the program no longer asks
                            // for leaves the list at its next save.
                            self.compile_unlisted(&e.file, &e.entry, e.stage)?;
                            progress.compiled.fetch_add(1, Ordering::Relaxed);
                        }
                    })
                })
                .collect();
            workers.into_iter().try_for_each(|w| {
                w.join().unwrap_or_else(|_| {
                    Err(GpuError::Shader("a shader warm-up thread panicked".into()))
                })
            })
        })?;
        Ok(missing.len())
    }

    /// The options in the cached file's name, so that optimised and debug builds, or a
    /// `FORGE_FP_PRECISE` run, keep their own SPIR-V side by side.
    fn variant(&self) -> &'static str {
        match (
            self.optimize,
            std::env::var_os("FORGE_FP_PRECISE").is_some(),
        ) {
            (true, false) => "O2",
            (true, true) => "O2p",
            (false, false) => "O0",
            (false, true) => "O0p",
        }
    }

    fn cached_path(&self, file: &str, entry: &str, hash: u64) -> PathBuf {
        self.cache_dir.join(format!(
            "{}@{entry}@{}@{hash:016x}.spv",
            file.trim_end_matches(".slang"),
            self.variant()
        ))
    }

    /// Removes `entry`'s SPIR-V of other keys in its variant, once `keep` is written: one file
    /// per entry and variant stays (#209).
    fn prune(&self, file: &str, entry: &str, keep: &Path) {
        let prefix = format!(
            "{}@{entry}@{}@",
            file.trim_end_matches(".slang"),
            self.variant()
        );
        let Some(dir) = keep.parent() else {
            return;
        };
        let Ok(listing) = fs::read_dir(dir) else {
            return;
        };
        for path in listing.filter_map(|e| e.ok().map(|e| e.path())) {
            let stale = path != keep
                && path.extension() == Some(OsStr::new("spv"))
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix));
            if stale {
                let _ = fs::remove_file(&path);
            }
        }
    }

    /// The key of `file`'s entries: a hash of it and of every file it imports, directly or not,
    /// with the compiler's version and options. Editing a shader recompiles only the entries
    /// of the files that reach it (#209). An import that names no file (a module of Slang's
    /// own) counts by its name.
    fn source_hash(&self, file: &str) -> Result<u64> {
        let mut sources: BTreeMap<String, Option<Vec<u8>>> = BTreeMap::new();
        let mut pending = vec![file.to_owned()];
        while let Some(name) = pending.pop() {
            if sources.contains_key(&name) {
                continue;
            }
            let text = match fs::read(self.shader_dir.join(&name)) {
                Ok(text) => Some(text),
                Err(error) if name == file => return Err(error.into()),
                Err(_) => None,
            };
            if let Some(text) = &text {
                pending.extend(imports(
                    &self.shader_dir,
                    &name,
                    &String::from_utf8_lossy(text),
                ));
            }
            sources.insert(name, text);
        }
        let mut hasher = Xxh3::new();
        for (name, text) in &sources {
            hasher.update(name.as_bytes());
            match text {
                Some(text) => {
                    hasher.update(&(text.len() as u64).to_le_bytes());
                    hasher.update(text);
                }
                None => hasher.update(&u64::MAX.to_le_bytes()),
            }
        }
        hasher.update(self.version.as_bytes());
        hasher.update(&[u8::from(self.optimize)]);
        hasher.update(&[u8::from(std::env::var_os("FORGE_FP_PRECISE").is_some())]);
        Ok(hasher.digest())
    }

    /// Compiles `entry` of `file` (relative to the shader directory) into SPIR-V words.
    pub fn compile(&self, file: &str, entry: &str, stage: ShaderStage) -> Result<Vec<u32>> {
        let request = ShaderEntry {
            file: file.to_owned(),
            entry: entry.to_owned(),
            stage,
        };
        if let Ok(mut list) = self.requested.lock()
            && !list.contains(&request)
        {
            list.push(request);
        }
        self.compile_unlisted(file, entry, stage)
    }

    /// [`Self::compile`] without listing the entry as asked for (the warm-up's compiles).
    fn compile_unlisted(&self, file: &str, entry: &str, stage: ShaderStage) -> Result<Vec<u32>> {
        let hash = self.source_hash(file)?;
        let cached = self.cached_path(file, entry, hash);
        if let Ok(bytes) = fs::read(&cached)
            && bytes.len() % 4 == 0
            && !bytes.is_empty()
        {
            return Ok(bytemuck::cast_slice(&bytes).to_vec());
        }
        let source = self.shader_dir.join(file);
        let mut cmd = Command::new(&self.slangc);
        cmd.arg(&source)
            .args(["-target", "spirv", "-profile", "spirv_1_6"])
            .args(["-entry", entry, "-stage", stage.slang_name()])
            .arg(if self.optimize { "-O2" } else { "-O0" })
            .args(["-fvk-use-entrypoint-name", "-matrix-layout-column-major"])
            // 39001: the bindless set aliases one binding under several resource types on purpose.
            .args(["-warnings-disable", "39001"])
            .arg("-I")
            .arg(&self.shader_dir)
            .arg("-o")
            .arg(&cached);
        if !self.optimize {
            cmd.arg("-g2");
        }
        if std::env::var_os("FORGE_FP_PRECISE").is_some() {
            // Debugging aid (issue #71): no contraction into FMAs, which the driver may otherwise
            // do differently from one draw to the next.
            cmd.args(["-fp-mode", "precise"]);
        }
        let output = cmd
            .output()
            .map_err(|e| GpuError::Shader(format!("cannot run slangc: {e}")))?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            let _ = fs::remove_file(&cached);
            return Err(GpuError::Shader(format!(
                "{file}:{entry} failed:\n{stderr}"
            )));
        }
        for line in stderr
            .lines()
            .filter(|l| l.contains("warning") && !l.contains("implicitly upgraded"))
        {
            tracing::warn!(shader = file, "{line}");
        }
        let bytes = fs::read(&cached)?;
        if bytes.len() % 4 != 0 || bytes.is_empty() {
            return Err(GpuError::Shader(format!(
                "{file}:{entry}: slangc produced no SPIR-V"
            )));
        }
        self.prune(file, entry, &cached);
        tracing::info!(shader = file, entry, "compiled");
        Ok(bytemuck::cast_slice(&bytes).to_vec())
    }
}

impl Device {
    /// Creates a shader module from SPIR-V words.
    pub fn create_shader_module(&self, spirv: &[u32], name: &str) -> Result<vk::ShaderModule> {
        let info = vk::ShaderModuleCreateInfo::default().code(spirv);
        // SAFETY: `spirv` is a complete SPIR-V module (validated by slangc).
        let module = unsafe { self.raw().create_shader_module(&info, None)? };
        self.set_name(module, name);
        Ok(module)
    }

    /// Destroys a shader module (pipelines keep their own copy of the code).
    pub fn destroy_shader_module(&self, module: vk::ShaderModule) {
        // SAFETY: modules may be destroyed as soon as the pipelines using them are created.
        unsafe { self.raw().destroy_shader_module(module, None) };
    }
}

/// How far a warm-up has gone ([`ShaderCompiler::warm_counted`], #200): the entries it found
/// the cache lacking (`usize::MAX` until it has looked) and those of them compiled so far.
#[derive(Debug)]
pub struct WarmProgress {
    /// The entries the cache lacks.
    pub missing: AtomicUsize,
    /// Those compiled so far.
    pub compiled: AtomicUsize,
}

impl Default for WarmProgress {
    fn default() -> Self {
        Self {
            missing: AtomicUsize::new(usize::MAX),
            compiled: AtomicUsize::new(0),
        }
    }
}

impl WarmProgress {
    /// The share compiled, 0 to 1, once it has looked and found some to compile; else none.
    pub fn share(&self) -> Option<f32> {
        let missing = self.missing.load(Ordering::Relaxed);
        (missing != usize::MAX && missing > 0)
            .then(|| self.compiled.load(Ordering::Relaxed) as f32 / missing as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_lists_read_back_and_skip_what_they_cannot_read() {
        let entries = vec![
            ShaderEntry {
                file: "meshlet.slang".into(),
                entry: "mesh_main".into(),
                stage: ShaderStage::Mesh,
            },
            ShaderEntry {
                file: "sky.slang".into(),
                entry: "irradiance_main".into(),
                stage: ShaderStage::Compute,
            },
        ];
        let text = format_entries(&entries);
        assert_eq!(parse_entries(&text), entries);
        // A line with an unknown stage or missing words is skipped; the rest are kept.
        let text = format!("{text}bad.slang main geometry\nhalf.slang\n");
        assert_eq!(parse_entries(&text), entries);
    }

    /// A shader directory of its own under the temporary directory, with these files.
    fn shader_dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = env::temp_dir().join(format!("forge-shader-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (file, text) in files {
            let path = dir.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }

    #[test]
    fn imports_resolve_modules_and_includes() {
        let dir = shader_dir(
            "imports",
            &[("lib/sky-view.slang", ""), ("lib/inc.slang", "")],
        );
        let text = "import bindless;\n  import lib.sky_view;\n// import gone;\n\
                    #include \"inc.slang\"\nfloat import_me;\n";
        assert_eq!(
            imports(&dir, "lib/main.slang", text),
            ["bindless.slang", "lib/sky-view.slang", "lib/inc.slang"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_key_follows_only_what_it_imports() {
        let dir = shader_dir(
            "keys",
            &[
                ("a.slang", "import b;\nvoid a() {}"),
                ("b.slang", "import c;\nvoid b() {}"),
                ("c.slang", "void c() {}"),
                ("d.slang", "import std;\nvoid d() {}"),
            ],
        );
        let compiler = ShaderCompiler {
            slangc: PathBuf::new(),
            shader_dir: dir.clone(),
            cache_dir: dir.clone(),
            optimize: true,
            version: "test".into(),
            requested: Arc::default(),
        };
        let keys = |names: &[&str]| -> Vec<u64> {
            names
                .iter()
                .map(|n| compiler.source_hash(n).unwrap())
                .collect()
        };
        let before = keys(&["a.slang", "b.slang", "c.slang", "d.slang"]);
        // c is reached by a through b: all three move; d does not.
        fs::write(dir.join("c.slang"), "void c() { }").unwrap();
        let after = keys(&["a.slang", "b.slang", "c.slang", "d.slang"]);
        assert!((0..3).all(|i| before[i] != after[i]));
        assert_eq!(before[3], after[3]);
        // A missing file is an error; a missing import (Slang's own module) is not.
        assert!(compiler.source_hash("gone.slang").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clones_share_the_entries_asked_for() {
        let compiler = ShaderCompiler {
            slangc: PathBuf::new(),
            shader_dir: PathBuf::new(),
            cache_dir: PathBuf::new(),
            optimize: true,
            version: String::new(),
            requested: Arc::default(),
        };
        let worker = compiler.clone();
        // The compile fails (no such file); the entry is still listed, on both.
        let _ = worker.compile("none.slang", "main", ShaderStage::Compute);
        assert_eq!(compiler.requested().len(), 1);
    }
}
