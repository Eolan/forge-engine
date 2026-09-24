use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Compiles Slang to SPIR-V with `slangc`, caching by content hash.
pub struct ShaderCompiler {
    slangc: PathBuf,
    shader_dir: PathBuf,
    cache_dir: PathBuf,
    optimize: bool,
    version: String,
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
        })
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
        Ok(hasher.digest())
    }

    /// Compiles `entry` of `file` (relative to the shader directory) into SPIR-V words.
    pub fn compile(&self, file: &str, entry: &str, stage: ShaderStage) -> Result<Vec<u32>> {
        let hash = self.source_hash()?;
        let cached = self.cache_dir.join(format!(
            "{}@{entry}@{hash:016x}.spv",
            file.trim_end_matches(".slang")
        ));
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
