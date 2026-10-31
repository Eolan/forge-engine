// Forge's C shim over NVIDIA's NRC (Neural Radiance Cache) for Vulkan (D-088, the trial): the
// library's API is C++ classes (`nrc::vulkan::Context`), so this file gives Rust a few plain C
// functions over them, which `crates/forge-gpu/src/nrc.rs` loads at run time. It is Forge's own
// code (MIT/Apache); `tools/fetch-nrc.sh` builds it against the git-ignored `nrc-sdk/` (NRC's
// headers and `NRC_Vulkan.lib`, the NVIDIA RTX SDKs licence) into `nrc-sdk/bin/forge_nrc.dll`.
// Nothing of NRC is copied here.
//
// The structures below are mirrored in `nrc.rs`; the sizes are checked at the foot of this file
// and on the Rust side. `NRC_Vulkan.dll` is delay-loaded (`tools/fetch-nrc.sh`): `forge_nrc_create`
// checks its signature, then loads it from the shim's folder, before any of its code runs.

#include <cstdint>
#include <cstring>
#include <string>

#include <NrcSecurity.h>
#include <NrcVk.h>

#define FORGE_NRC_API extern "C" __declspec(dllexport)

// `nrc::ContextSettings`, the settings a change of which reconfigures the cache.
struct ForgeNrcContext
{
    uint32_t learn_irradiance;
    uint32_t include_direct;
    uint32_t request_reset;
    uint32_t samples_per_pixel;
    float bounds_min[3];
    float feature_size;
    float bounds_max[3];
    uint32_t max_path_vertices;
    uint32_t frame[2];
    uint32_t training[2];
};

// `nrc::FrameSettings`, set every frame.
struct ForgeNrcFrame
{
    float max_average_radiance;
    float termination;
    float training_termination;
    float learning_rate;
    uint32_t used_training[2];
    uint32_t training_iterations;
    uint32_t train;
    uint32_t skip_delta_vertices;
    float proportion_unbiased;
    float self_training_attenuation;
    uint32_t resolve_mode;
    float proportion_primary;
};

// One of NRC's buffers (`nrc::vulkan::BufferInfo`), as the library allocated it.
struct ForgeNrcBuffer
{
    uint64_t buffer;
    uint64_t size;
    uint64_t offset;
    uint64_t address;
};

typedef void (*ForgeNrcLog)(const char* message, int level);

static ForgeNrcLog g_log = nullptr;

static void forward_log(const char* message, nrc::LogLevel level)
{
    if (g_log)
        g_log(message, (int)level);
}

static nrc::ContextSettings context_settings(const ForgeNrcContext& c)
{
    nrc::ContextSettings s;
    s.learnIrradiance = c.learn_irradiance != 0;
    s.includeDirectLighting = c.include_direct != 0;
    s.requestReset = c.request_reset != 0;
    s.sceneBoundsMin = {c.bounds_min[0], c.bounds_min[1], c.bounds_min[2]};
    s.sceneBoundsMax = {c.bounds_max[0], c.bounds_max[1], c.bounds_max[2]};
    s.smallestResolvableFeatureSize = c.feature_size;
    s.frameDimensions = {c.frame[0], c.frame[1]};
    s.trainingDimensions = {c.training[0], c.training[1]};
    s.samplesPerPixel = c.samples_per_pixel;
    s.maxPathVertices = c.max_path_vertices;
    return s;
}

// NRC's version (major << 16 | minor) as this shim was built against: the Rust side refuses
// another.
FORGE_NRC_API uint32_t forge_nrc_version()
{
    return (NRC_VERSION_MAJOR << 16) | NRC_VERSION_MINOR;
}

// The size of `NrcConstants`, the shaders' block: the Rust side checks it.
FORGE_NRC_API uint32_t forge_nrc_constants_size()
{
    return (uint32_t)sizeof(NrcConstants);
}

// The training paths' size for frames of `width` × `height` and `iterations` training batches a
// frame (0: NRC's default), `nrc::ComputeIdealTrainingDimensions`.
FORGE_NRC_API void forge_nrc_training_size(uint32_t width, uint32_t height, uint32_t iterations, uint32_t* out)
{
    nrc_uint2 size = nrc::ComputeIdealTrainingDimensions({width, height}, iterations);
    out[0] = size.x;
    out[1] = size.y;
}

// NRC's folder, kept for the library's life: NRC keeps the pointer it is given
// (`GlobalSettings::depsDirectoryPath`).
static std::wstring g_dir;

// Checks `NRC_Vulkan.dll`'s signature in `dir` (NRC's `VerifySignature`), loads it from there,
// initializes the library (its CUDA dependencies from `dir`, `frames_in_flight` frames) and creates
// a context on the device. 0 and `*out` on success, else NRC's status (100 for a bad signature,
// 101 for a library that does not load).
FORGE_NRC_API int forge_nrc_create(VkInstance instance, VkPhysicalDevice gpu, VkDevice device, const wchar_t* dir, uint32_t frames_in_flight, ForgeNrcLog log, void** out)
{
    *out = nullptr;
    g_log = log;
    g_dir = dir;
    std::wstring library = g_dir + L"\\NRC_Vulkan.dll";
    if (!nrc::security::VerifySignature(library.c_str()))
        return 100;
    // Loaded here, by its path, so that the delay-load's lookup by name finds this one.
    if (!LoadLibraryExW(library.c_str(), nullptr, LOAD_WITH_ALTERED_SEARCH_PATH))
        return 101;
    nrc::GlobalSettings global;
    global.loggerFn = &forward_log;
    global.enableGPUMemoryAllocation = true;
    // On (0.5 MB at 1080p): without its debug buffer, NRC 0.16 still writes that buffer's
    // descriptor, a null handle (the validation layer's VUID-VkDescriptorBufferInfo-buffer-02998).
    global.enableDebugBuffers = true;
    global.maxNumFramesInFlight = frames_in_flight;
    global.depsDirectoryPath = g_dir.c_str();
    nrc::Status status = nrc::vulkan::Initialize(global);
    if (status != nrc::Status::OK)
        return (int)status;
    nrc::vulkan::Context* context = nullptr;
    status = nrc::vulkan::Context::Create(device, gpu, instance, context);
    if (status != nrc::Status::OK)
    {
        nrc::vulkan::Shutdown();
        return (int)status;
    }
    *out = context;
    return 0;
}

// Destroys the context and shuts the library down (the device idle).
FORGE_NRC_API void forge_nrc_destroy(void* context)
{
    if (context)
        nrc::vulkan::Context::Destroy(*(nrc::vulkan::Context*)context);
    nrc::vulkan::Shutdown();
    g_log = nullptr;
}

// (Re)configures the cache for `settings` (the library allocates its buffers) and writes its
// buffers, `nrc::BufferIdx` order, into `buffers` (`count` of them at most). 0 on success.
FORGE_NRC_API int forge_nrc_configure(void* context, const ForgeNrcContext* settings, ForgeNrcBuffer* buffers, uint32_t count)
{
    nrc::vulkan::Context* c = (nrc::vulkan::Context*)context;
    nrc::Status status = c->Configure(context_settings(*settings));
    if (status != nrc::Status::OK)
        return (int)status;
    const nrc::vulkan::Buffers* b = c->GetBuffers();
    for (uint32_t i = 0; i < count && i < (uint32_t)nrc::BufferIdx::Count; i++)
    {
        const nrc::vulkan::BufferInfo& info = b->buffers[i];
        buffers[i].buffer = (uint64_t)info.resource;
        buffers[i].size = info.allocatedSize;
        buffers[i].offset = info.allocatedOffset;
        buffers[i].address = info.deviceAddress;
    }
    return 0;
}

// Records NRC's start of a frame into `cmd` (its counters cleared) and writes the shaders'
// `NrcConstants` for it into `constants` (`forge_nrc_constants_size` bytes). 0 on success.
FORGE_NRC_API int forge_nrc_begin_frame(void* context, VkCommandBuffer cmd, const ForgeNrcFrame* frame, void* constants)
{
    nrc::vulkan::Context* c = (nrc::vulkan::Context*)context;
    nrc::FrameSettings s;
    s.maxExpectedAverageRadianceValue = frame->max_average_radiance;
    s.terminationHeuristicThreshold = frame->termination;
    s.trainingTerminationHeuristicThreshold = frame->training_termination;
    s.learningRate = frame->learning_rate;
    s.usedTrainingDimensions = {frame->used_training[0], frame->used_training[1]};
    s.numTrainingIterations = frame->training_iterations;
    s.trainTheCache = frame->train != 0;
    s.skipDeltaVertices = frame->skip_delta_vertices != 0;
    s.proportionUnbiased = frame->proportion_unbiased;
    s.selfTrainingAttenuation = frame->self_training_attenuation;
    s.resolveMode = (NrcResolveMode)frame->resolve_mode;
    s.proportionPrimarySegmentsToTrainOn = frame->proportion_primary;
    nrc::Status status = c->BeginFrame(cmd, s);
    if (status != nrc::Status::OK)
        return (int)status;
    NrcConstants out;
    status = c->PopulateShaderConstants(out);
    if (status != nrc::Status::OK)
        return (int)status;
    std::memcpy(constants, &out, sizeof(out));
    return 0;
}

// Records NRC's inference at this frame's queries and its training on this frame's paths into
// `cmd` (no training loss: its read back needs a CUDA stream the Vulkan path does not have).
FORGE_NRC_API int forge_nrc_query_and_train(void* context, VkCommandBuffer cmd)
{
    return (int)((nrc::vulkan::Context*)context)->QueryAndTrain(cmd, nullptr);
}

// NRC's end of a frame, after its command buffer was submitted to `queue`.
FORGE_NRC_API int forge_nrc_end_frame(void* context, VkQueue queue)
{
    return (int)((nrc::vulkan::Context*)context)->EndFrame(queue);
}

static_assert(sizeof(ForgeNrcContext) == 64, "ForgeNrcContext mirrors nrc.rs");
static_assert(sizeof(ForgeNrcFrame) == 52, "ForgeNrcFrame mirrors nrc.rs");
static_assert(sizeof(ForgeNrcBuffer) == 32, "ForgeNrcBuffer mirrors nrc.rs");
static_assert(sizeof(NrcConstants) == 96, "NrcConstants as nrc.rs expects");
