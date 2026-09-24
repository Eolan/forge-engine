/// Errors of the GPU layer.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    /// A Vulkan call failed.
    #[error("Vulkan: {0}")]
    Vulkan(#[from] ash::vk::Result),
    /// The Vulkan loader could not be found.
    #[error("Vulkan loader: {0}")]
    Loading(#[from] ash::LoadingError),
    /// GPU memory allocation failed.
    #[error("GPU memory: {0}")]
    Allocation(#[from] gpu_allocator::AllocationError),
    /// Shader compilation failed.
    #[error("shader: {0}")]
    Shader(String),
    /// The machine lacks something the engine requires.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// File access failed.
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Result alias for the GPU layer.
pub type Result<T> = std::result::Result<T, GpuError>;
