use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

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

/// Compiles Slang to SPIR-V with `slangc`, caching by content hash.
pub struct ShaderCompiler {
    slangc: PathBuf,
    shader_dir: PathBuf,
    cache_dir: PathBuf,
    optimize: bool,
    version: String,
    /// Every entry asked for so far, in order (issue #25).
    requested: Mutex<Vec<ShaderEntry>>,
}

/// A copy for another thread, with an empty request list.
impl Clone for ShaderCompiler {
    fn clone(&self) -> Self {
        Self {
            slangc: self.slangc.clone(),
            shader_dir: self.shader_dir.clone(),
            cache_dir: self.cache_dir.clone(),
            optimize: self.optimize,
            version: self.version.clone(),
            requested: Mutex::new(Vec::new()),
        }
    }
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
        Ok(Self {
            slangc,
            shader_dir: shader_dir.into(),
            cache_dir,
            optimize,
            version,
            requested: Mutex::new(Vec::new()),
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
        let hash = self.source_hash()?;
        let missing: Vec<&ShaderEntry> = entries
            .iter()
            .filter(|e| !self.cached_path(&e.file, &e.entry, hash).exists())
            .collect();
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
                            self.compile(&e.file, &e.entry, e.stage)?;
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

    fn cached_path(&self, file: &str, entry: &str, hash: u64) -> PathBuf {
        self.cache_dir.join(format!(
            "{}@{entry}@{hash:016x}.spv",
            file.trim_end_matches(".slang")
        ))
    }

    fn source_hash(&self) -> Result<u64> {
        // Hash every .slang file: modules import each other, so any change invalidates all.
        let mut files: Vec<PathBuf> = fs::read_dir(&self.shader_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension() == Some(OsStr::new("slang")))
            .collect();
        files.sort();
        let mut hasher = Xxh3::new();
        for file in files {
            hasher.update(
                file.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .as_bytes(),
            );
            hasher.update(&fs::read(&file)?);
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
        let hash = self.source_hash()?;
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
}
