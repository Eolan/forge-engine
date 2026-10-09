//! The loading screen (issue #25). A demo started with [`crate::run_loading`] does its heavy
//! CPU work (procedural meshes, cluster DAGs) on a thread of its own while the shell keeps the
//! window alive: this stage draws a hammer striking an anvil in the bottom right corner, and a
//! bar of the shaders compiled ahead at the bottom centre (#200, `shaders/loading.slang`); the
//! frames it draws do not count ([`Context::loading`]). When the thread is done, the step it
//! returns finishes the demo (the uploads, the pipelines) on a worker with the shell's
//! [`Setup`], the loading screen drawing on meanwhile (#201: on the main thread it froze the
//! screen for seconds; the device's queues are held by whoever uses them). Then the demo takes
//! over from frame 0 with a fresh profile, as if it had been built before the first frame.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::Result;
use forge_gpu::{
    FullscreenPipelineDesc, ImageAccess, Pipeline, ShaderCompiler, ShaderStage, WarmProgress, vk,
};
use winit::keyboard::KeyCode;

use crate::{Context, Demo, FrameInfo, Input, Profile, Setup};

/// What a demo's preparation hands back: the step that finishes it (uploads, pipelines), with
/// the shell's [`Setup`]. It runs on a worker while the loading screen goes on drawing (#201);
/// before, it ran on the main thread, and the screen froze for its seconds.
pub type Finish<D> = Box<dyn FnOnce(&Setup) -> Result<D> + Send>;

/// How long a loading frame lasts at least: the preparation needs the cores more than the
/// animation needs a high frame rate.
const LOADING_FRAME: Duration = Duration::from_millis(8);

/// Threads compiling shaders ahead: `slangc` is a process per entry point.
const SHADER_THREADS: usize = 4;

/// The loading screen, then the demo.
pub(crate) enum Stage<D> {
    Loading {
        pipeline: Pipeline,
        started: Instant,
        thread: Option<JoinHandle<Result<Finish<D>>>>,
        /// Compiles the shaders the previous run asked for into the cache, while the loading
        /// screen shows: after a shader change, the finishing step finds them there.
        warm_up: Option<JoinHandle<forge_gpu::Result<usize>>>,
        /// How far it has gone: the bar at the bottom of the screen (#200).
        progress: Arc<WarmProgress>,
        /// Where the program's shader entries are listed for the next start.
        entries: PathBuf,
        /// The finishing step on its worker once the preparation is done (#201), the
        /// milliseconds the preparation took, and the frame's size the step was given.
        finishing: Option<Box<Finishing<D>>>,
        /// When the last loading frame was asked for, and the longest wait between two: the
        /// screen's longest freeze (#201).
        last_frame: Instant,
        longest_gap: Duration,
    },
    Running(D),
    /// The preparation or the finishing step failed: the next frame returns the error.
    Failed(Option<anyhow::Error>),
}

/// The finishing step on its worker (#201): its thread, the milliseconds the preparation took
/// before it, and the frame's size it was given.
pub(crate) struct Finishing<D> {
    worker: JoinHandle<Result<D>>,
    prepared_ms: u128,
    given: vk::Extent2D,
}

/// Whether a thread is done (or was never started).
fn finished<T>(handle: &Option<JoinHandle<T>>) -> bool {
    handle.as_ref().is_none_or(JoinHandle::is_finished)
}

impl<D: Demo + Send> Stage<D> {
    /// Starts `prepare` on its thread and the loading screen in the window.
    pub(crate) fn start(
        ctx: &mut Context,
        prepare: impl FnOnce() -> Result<Finish<D>> + Send + 'static,
    ) -> Result<Self> {
        let vertex = ctx.device.create_shader_module(
            &ctx.shaders
                .compile("loading.slang", "vert_main", ShaderStage::Vertex)?,
            "loading vs",
        )?;
        let fragment = ctx.device.create_shader_module(
            &ctx.shaders
                .compile("loading.slang", "frag_main", ShaderStage::Fragment)?,
            "loading fs",
        )?;
        let pipeline = ctx
            .device
            .create_fullscreen_pipeline(&FullscreenPipelineDesc {
                vertex: (vertex, "vert_main"),
                fragment: (fragment, "frag_main"),
                color_formats: &[ctx.swapchain.format()],
                push_constant_bytes: 16,
                alpha_blend: false,
                depth_test: None,
                depth_write: false,
                name: "loading",
            });
        ctx.device.destroy_shader_module(vertex);
        ctx.device.destroy_shader_module(fragment);
        let pipeline = pipeline?;
        let thread = std::thread::Builder::new()
            .name("loading".to_owned())
            .spawn(prepare)?;
        let program = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "program".to_owned());
        let entries = ctx.shaders.cache_dir().join(format!("{program}.entries"));
        let listed = ShaderCompiler::load_entries(&entries);
        let compiler = ctx.shaders.clone();
        let progress = Arc::new(WarmProgress::default());
        let counted = progress.clone();
        let warm_up = std::thread::Builder::new()
            .name("shader warm-up".to_owned())
            .spawn(move || compiler.warm_counted(&listed, SHADER_THREADS, &counted))?;
        ctx.loading = true;
        Ok(Self::Loading {
            pipeline,
            started: Instant::now(),
            thread: Some(thread),
            warm_up: Some(warm_up),
            progress,
            entries,
            finishing: None,
            last_frame: Instant::now(),
            longest_gap: Duration::ZERO,
        })
    }

    /// Once the preparation is done, starts the finishing step on its worker (#201); once that
    /// is done, hands the demo the frames.
    fn poll(&mut self, ctx: &mut Context) {
        let Self::Loading {
            started,
            thread,
            warm_up,
            entries,
            finishing,
            last_frame,
            longest_gap,
            ..
        } = self
        else {
            return;
        };
        let now = Instant::now();
        *longest_gap = (*longest_gap).max(now - *last_frame);
        *last_frame = now;
        if let Some(step) = finishing.as_ref() {
            if !step.worker.is_finished() {
                std::thread::sleep(LOADING_FRAME);
                return;
            }
            let Finishing {
                worker,
                prepared_ms,
                given,
            } = *finishing.take().expect("a finishing step");
            let result = worker
                .join()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("the finishing step panicked")));
            let (started, entries, gap) = (*started, entries.clone(), *longest_gap);
            tracing::info!(
                longest_gap_ms = gap.as_millis(),
                "the loading screen's longest wait between two frames (#201)"
            );
            self.finish(ctx, result, prepared_ms, started, given, &entries);
            return;
        }
        if !finished(thread) || !finished(warm_up) {
            std::thread::sleep(LOADING_FRAME);
            return;
        }
        match warm_up.take().map(JoinHandle::join) {
            Some(Ok(Ok(compiled))) if compiled > 0 => {
                tracing::info!(compiled, "shaders compiled ahead behind the loading screen");
            }
            Some(Ok(Err(error))) => tracing::warn!(%error, "shader warm-up"),
            _ => {}
        }
        let prepared = thread.take().map(JoinHandle::join);
        let prepared_ms = started.elapsed().as_millis();
        let finish = match prepared {
            Some(Ok(Ok(finish))) => finish,
            Some(Ok(Err(error))) => {
                *self = Self::Failed(Some(error));
                return;
            }
            _ => {
                *self = Self::Failed(Some(anyhow::anyhow!("the loading thread panicked")));
                return;
            }
        };
        let setup = ctx.setup();
        let given = setup.extent();
        match std::thread::Builder::new()
            .name("finishing".to_owned())
            .spawn(move || finish(&setup))
        {
            Ok(worker) => {
                *finishing = Some(Box::new(Finishing {
                    worker,
                    prepared_ms,
                    given,
                }))
            }
            Err(error) => *self = Self::Failed(Some(error.into())),
        }
    }

    /// The finishing step's `result`: the demo takes the frames from frame 0, or the next frame
    /// returns the error.
    fn finish(
        &mut self,
        ctx: &mut Context,
        result: Result<D>,
        prepared_ms: u128,
        started: Instant,
        given: vk::Extent2D,
        entries: &Path,
    ) {
        // The loading frames still in flight use the pipeline this drops.
        ctx.device.wait_idle();
        ctx.loading = false;
        ctx.profile = Profile::new(ctx.profile.mode);
        *self = match result {
            Ok(mut demo) => {
                if let Err(error) = ctx.shaders.save_entries(entries) {
                    tracing::warn!(%error, "cannot list the shader entries for the next start");
                }
                tracing::info!(
                    prepared_ms,
                    total_ms = started.elapsed().as_millis(),
                    "loaded"
                );
                // The window changed size while the step worked with the size it was given.
                let now = ctx.extent();
                if (now.width, now.height) != (given.width, given.height)
                    && let Err(error) = demo.resized(ctx)
                {
                    *self = Self::Failed(Some(error));
                    return;
                }
                Self::Running(demo)
            }
            Err(error) => Self::Failed(Some(error)),
        };
    }
}

impl<D: Demo + Send> Demo for Stage<D> {
    fn resized(&mut self, ctx: &mut Context) -> Result<()> {
        match self {
            Self::Running(demo) => demo.resized(ctx),
            _ => Ok(()),
        }
    }

    fn key_pressed(&mut self, ctx: &mut Context, code: KeyCode) {
        if let Self::Running(demo) = self {
            demo.key_pressed(ctx, code);
        }
    }

    fn update(&mut self, ctx: &mut Context, input: &Input, dt: f32) {
        self.poll(ctx);
        if let Self::Running(demo) = self {
            demo.update(ctx, input, dt);
        }
    }

    fn render<'f>(&'f mut self, ctx: &mut Context, frame: &mut FrameInfo<'f>) -> Result<()> {
        match self {
            Self::Running(demo) => demo.render(ctx, frame),
            Self::Failed(error) => Err(error
                .take()
                .unwrap_or_else(|| anyhow::anyhow!("the demo failed to start"))),
            Self::Loading {
                pipeline,
                started,
                progress,
                ..
            } => {
                // The window's own size: the loading screen is not supersampled.
                let extent = ctx.swapchain.extent();
                let push = [
                    extent.width as f32,
                    extent.height as f32,
                    started.elapsed().as_secs_f32(),
                    // The shaders compiled ahead, or none to compile: no bar.
                    progress.share().unwrap_or(-1.0),
                ];
                let pipeline = &*pipeline;
                let target = frame.target;
                frame
                    .graph
                    .pass("app/loading")
                    .image(target, ImageAccess::ColorAttachment)
                    .run(move |resources, commands| {
                        let attachments = [vk::RenderingAttachmentInfo::default()
                            .image_view(resources.view(target))
                            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                            .load_op(vk::AttachmentLoadOp::DONT_CARE)
                            .store_op(vk::AttachmentStoreOp::STORE)];
                        let info = vk::RenderingInfo::default()
                            .render_area(vk::Rect2D {
                                offset: vk::Offset2D::default(),
                                extent,
                            })
                            .layer_count(1)
                            .color_attachments(&attachments);
                        commands.begin_rendering(&info);
                        commands.bind_pipeline(pipeline);
                        commands.set_viewport_full(extent);
                        commands.push_constants(pipeline, &push);
                        commands.draw(3, 1);
                        commands.end_rendering();
                        Ok(())
                    });
                Ok(())
            }
        }
    }

    fn title(&mut self, ctx: &Context) -> Option<String> {
        match self {
            Self::Running(demo) => demo.title(ctx),
            _ => None,
        }
    }
}
