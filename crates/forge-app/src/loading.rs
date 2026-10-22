//! The loading screen (issue #25). A demo started with [`crate::run_loading`] does its heavy
//! CPU work (procedural meshes, cluster DAGs) on a thread of its own while the shell keeps the
//! window alive: this stage draws a hammer striking an anvil in the bottom right corner, and a
//! bar of the shaders compiled ahead at the bottom left (#200, `shaders/loading.slang`); the
//! shell writes over it what the engine is busy with ([`layout_activity`], from
//! `forge_core::activity`): the shaders compiling over the bar, the longest tasks running and
//! the last step done beside the forge. The frames it draws do not count ([`Context::loading`]). When the thread is done, the step it
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
    FRAMES_IN_FLIGHT, FullscreenPipelineDesc, ImageAccess, Pipeline, ShaderCompiler, ShaderStage,
    WarmProgress, vk,
};
use winit::keyboard::KeyCode;

use crate::{Canvas, Color, Context, Demo, FrameInfo, Input, Profile, Setup};

/// What a demo's preparation hands back: the step that finishes it (uploads, pipelines), with
/// the shell's [`Setup`]. It runs on a worker while the loading screen goes on drawing (#201);
/// before, it ran on the main thread, and the screen froze for its seconds.
pub type Finish<D> = Box<dyn FnOnce(&Setup) -> Result<D> + Send>;

/// How long a loading frame lasts at least: the preparation needs the cores more than the
/// animation needs a high frame rate.
const LOADING_FRAME: Duration = Duration::from_millis(8);

/// Frames an interactive run draws behind the loading screen once the demo has started (#342):
/// its first frames create pipelines on first use, start the upscaler and update every probe as
/// young, and took 55–120 ms in a large town (frame 3 still 25–40 ms), a hitch as it appeared.
pub(crate) const SETTLE_FRAMES: u32 = 6;

/// Threads compiling shaders ahead, `slangc` being a process per entry point: all the cores but
/// two, four at least. Four left a fresh cache's 150 entries a quarter of the machine.
fn shader_threads() -> usize {
    std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .saturating_sub(2)
        .max(4)
}

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
        /// The frames the demo will draw behind the screen once started ([`SETTLE_FRAMES`]; 0 in
        /// scripted runs).
        settle: u32,
        /// While the preparation is done and the shaders compiled ahead are not: what the screen
        /// says it waits for (`forge_core::activity`).
        waiting: Option<forge_core::activity::Task>,
    },
    /// The demo started, its first frames drawn behind the loading screen (#342): `shown`
    /// frames left with the screen over them, then `linger` more before the screen's pipeline,
    /// which the frames in flight may still use, is dropped.
    Settling {
        demo: D,
        pipeline: Pipeline,
        started: Instant,
        progress: Arc<WarmProgress>,
        shown: u32,
        linger: u32,
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
        settle: u32,
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
            .spawn(move || {
                let _task = forge_core::activity::begin("app", "preparing the scene");
                prepare()
            })?;
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
            .spawn(move || {
                // A program's first start on this cache has no list: the repository's for it
                // (#345), else every entry the shaders declare, rather than its own one by one
                // as it asks for them (26 s of the village's 34 on a fresh cache).
                let listed = if listed.is_empty() {
                    let repository = compiler.listed_entries(&program);
                    if repository.is_empty() {
                        compiler.declared_entries()
                    } else {
                        repository
                    }
                } else {
                    listed
                };
                compiler.warm_counted(&listed, shader_threads(), &counted)
            })?;
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
            settle,
            waiting: None,
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
            waiting,
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
            if finished(thread) && waiting.is_none() {
                *waiting = Some(forge_core::activity::begin(
                    "app",
                    "waiting for the shaders compiled ahead",
                ));
            }
            std::thread::sleep(LOADING_FRAME);
            return;
        }
        *waiting = None;
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
            .spawn(move || {
                let _task =
                    forge_core::activity::begin("app", "finishing: GPU uploads and pipelines");
                finish(&setup)
            }) {
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
        // The frames drawn behind the screen (#342): not over an HDR or a supersampled frame,
        // whose targets the screen's pipeline does not draw into.
        let settle = match self {
            Self::Loading { settle, .. } if !ctx.output.is_hdr() && ctx.ssaa == 1 => *settle,
            _ => 0,
        };
        // The loading frames still in flight use the pipeline this drops.
        ctx.device.wait_idle();
        ctx.loading = false;
        ctx.profile = Profile::new(ctx.profile.mode);
        let next = match result {
            Ok(mut demo) => {
                if let Err(error) = ctx.shaders.save_entries(entries) {
                    tracing::warn!(%error, "cannot list the shader entries for the next start");
                }
                tracing::info!(
                    prepared_ms,
                    total_ms = started.elapsed().as_millis(),
                    "loaded"
                );
                // The load profiled (`dev.load_profile`, #345): its report in the log and in
                // the cache's profiles.
                if let Some(profile) = forge_core::activity::record_finish() {
                    tracing::info!("{}", profile.report());
                    let dir =
                        forge_core::derived::cache_dir(forge_core::derived::CacheKind::Profiles);
                    let program = std::env::current_exe()
                        .ok()
                        .and_then(|p| p.file_stem()?.to_str().map(str::to_owned))
                        .unwrap_or_else(|| "program".to_owned());
                    match profile.save(&dir, &program) {
                        Ok(path) => tracing::info!(path = %path.display(), "load profile saved"),
                        Err(error) => tracing::warn!(%error, "cannot save the load profile"),
                    }
                }
                // The window changed size while the step worked with the size it was given.
                let now = ctx.extent();
                if (now.width, now.height) != (given.width, given.height)
                    && let Err(error) = demo.resized(ctx)
                {
                    *self = Self::Failed(Some(error));
                    return;
                }
                // Its console variables, on the main thread now that it exists (D-069).
                demo.register(ctx);
                if settle > 0
                    && let Self::Loading {
                        pipeline,
                        started,
                        progress,
                        ..
                    } = std::mem::replace(self, Self::Failed(None))
                {
                    Self::Settling {
                        demo,
                        pipeline,
                        started,
                        progress,
                        shown: settle,
                        linger: FRAMES_IN_FLIGHT as u32 + 1,
                    }
                } else {
                    Self::Running(demo)
                }
            }
            Err(error) => Self::Failed(Some(error)),
        };
        *self = next;
    }
}

impl<D: Demo + Send> Demo for Stage<D> {
    fn resized(&mut self, ctx: &mut Context) -> Result<()> {
        match self {
            Self::Running(demo) | Self::Settling { demo, .. } => demo.resized(ctx),
            _ => Ok(()),
        }
    }

    fn key_pressed(&mut self, ctx: &mut Context, code: KeyCode) {
        if let Self::Running(demo) | Self::Settling { demo, .. } = self {
            demo.key_pressed(ctx, code);
        }
    }

    fn update(&mut self, ctx: &mut Context, input: &Input, dt: f32) {
        self.poll(ctx);
        // The screen's frames are done and none in flight uses its pipeline: the demo alone.
        if matches!(
            self,
            Self::Settling {
                shown: 0,
                linger: 0,
                ..
            }
        ) && let Self::Settling { demo, .. } = std::mem::replace(self, Self::Failed(None))
        {
            *self = Self::Running(demo);
        }
        if let Self::Running(demo) | Self::Settling { demo, .. } = self {
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
                draw_screen(ctx, frame, pipeline, *started, progress);
                Ok(())
            }
            Self::Settling {
                demo,
                pipeline,
                started,
                progress,
                shown,
                linger,
            } => {
                // The demo's frame, under the screen while it settles (#342).
                demo.render(ctx, frame)?;
                if *shown > 0 {
                    *shown -= 1;
                    draw_screen(ctx, frame, pipeline, *started, progress);
                } else {
                    *linger = linger.saturating_sub(1);
                }
                Ok(())
            }
        }
    }

    fn title(&mut self, ctx: &Context) -> Option<String> {
        match self {
            Self::Running(demo) | Self::Settling { demo, .. } => demo.title(ctx),
            _ => None,
        }
    }

    fn command(
        &mut self,
        ctx: &mut Context,
        name: &str,
        args: &[String],
    ) -> Option<std::result::Result<String, String>> {
        match self {
            Self::Running(demo) | Self::Settling { demo, .. } => demo.command(ctx, name, args),
            _ => None,
        }
    }
}

/// The shaders' bar's left end and half its length, shares of the screen's smaller side
/// (`BAR_LEFT`, `BAR_HALF` in `loading.slang`), and its height over the screen's foot.
const BAR_LEFT: f32 = 0.06;
const BAR_HALF: f32 = 0.16;
const BAR_UP: f32 = 0.06;
/// The forge's centre in from the bottom right corner and its size, shares of the smaller side
/// (`frag_main` in `loading.slang`).
const FORGE_IN: [f32; 2] = [0.14, 0.13];
const FORGE_SIZE: f32 = 0.16;
/// Tasks of the engine's running shown beside the forge, at most.
const SHOWN_TASKS: usize = 4;

/// What the engine is busy with ([`forge_core::activity`], the owner, 2026-10-21: "some loadings
/// are quite long and I would like to know what"), on `canvas` over the loading screen at
/// `extent`: above the shaders' bar their count and the entries compiling; beside the forge, to
/// its left, the longest tasks running and the last step done. Where the screen is too narrow
/// for them there, above the forge.
pub(crate) fn layout_activity(canvas: &mut Canvas, extent: vk::Extent2D) {
    let (cols, rows) = (canvas.cols(), canvas.rows());
    if cols < 8 || rows < 4 {
        return;
    }
    let activity = forge_core::activity::snapshot();
    let (w, h) = (extent.width as f32, extent.height as f32);
    let (cell_w, cell_h) = (w / cols as f32, h / rows as f32);
    let scale = w.min(h);
    let col_at = |x: f32| (x / cell_w).floor().max(0.0) as usize;
    let row_at = |y: f32| (y / cell_h).floor().clamp(0.0, (rows - 1) as f32) as usize;
    // Above the bar, from its left end: the count compiled ahead, then the entries compiling.
    let ahead = activity.running_in("shaders").find_map(|r| r.progress);
    let compiling: Vec<&str> = activity
        .running_in("shaders")
        .filter(|r| r.progress.is_none())
        .map(|r| r.label.as_str())
        .collect();
    let bar_left = col_at(BAR_LEFT * scale);
    let bar_row = row_at(h - BAR_UP * scale);
    if ahead.is_some() || !compiling.is_empty() {
        let mut line = match ahead {
            Some((done, total)) => format!("Compiling shaders {done}/{total}"),
            None => "Compiling shaders".to_owned(),
        };
        if !compiling.is_empty() {
            line += ": ";
            line += &compiling.join(", ");
        }
        let room = col_at(w * 0.5).saturating_sub(bar_left).max(16);
        canvas.text(
            bar_left,
            bar_row.saturating_sub(2),
            &fit(&line, room),
            Color::Grey,
        );
    }
    // To the forge's left: the longest tasks running, the last step done under them.
    let mut lines: Vec<(String, Color)> = activity
        .running
        .iter()
        .filter(|r| r.system != "shaders")
        .take(SHOWN_TASKS)
        .map(|r| {
            let count = r
                .progress
                .map(|(done, total)| format!(" {done}/{total}"))
                .unwrap_or_default();
            (
                format!("{}{count}  {}", capitalised(&r.label), seconds(r.elapsed)),
                Color::White,
            )
        })
        .collect();
    if let Some(d) = activity.done.iter().rev().find(|d| d.system != "shaders") {
        let took = d
            .took
            .map(|t| format!(" ({})", seconds(t)))
            .unwrap_or_default();
        lines.push((format!("done: {}{took}", d.label), Color::Dim));
    }
    if lines.is_empty() {
        return;
    }
    let forge_x = w - FORGE_IN[0] * scale;
    let forge_y = h - FORGE_IN[1] * scale;
    let right = col_at(forge_x - 0.75 * FORGE_SIZE * scale);
    let left = col_at((BAR_LEFT + 2.0 * BAR_HALF) * scale) + 2;
    let (right, room, last) = if right.saturating_sub(left) >= 24 {
        // Beside it, its lines about its middle, over the bar's text.
        let last = (row_at(forge_y) + lines.len() / 2).min(bar_row.saturating_sub(3));
        (right, right - left, last)
    } else {
        // Above it.
        let right = cols.saturating_sub(2);
        let last = row_at(forge_y - 0.7 * FORGE_SIZE * scale);
        (right, right.min(cols / 2), last)
    };
    let first = (last + 1).saturating_sub(lines.len());
    for (k, (text, colour)) in lines.iter().enumerate() {
        let text = fit(text, room);
        let col = right.saturating_sub(text.len());
        canvas.text(col, first + k, &text, *colour);
    }
}

/// `text` cut to `room` characters, "..." where cut.
fn fit(text: &str, room: usize) -> String {
    if text.chars().count() <= room {
        return text.to_owned();
    }
    let kept: String = text.chars().take(room.saturating_sub(3)).collect();
    kept + "..."
}

/// `text` with a capital first letter.
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// A duration as the screen shows it: "0.4 s", "12 s", "2 min 05 s".
fn seconds(d: Duration) -> String {
    let s = d.as_secs_f32();
    if s < 10.0 {
        format!("{s:.1} s")
    } else if s < 120.0 {
        format!("{s:.0} s")
    } else {
        format!("{} min {:02} s", d.as_secs() / 60, d.as_secs() % 60)
    }
}

/// The loading screen over `frame`'s target, at the window's own size (it is not supersampled).
fn draw_screen<'f>(
    ctx: &Context,
    frame: &mut FrameInfo<'f>,
    pipeline: &'f Pipeline,
    started: Instant,
    progress: &WarmProgress,
) {
    let extent = ctx.swapchain.extent();
    let push = [
        extent.width as f32,
        extent.height as f32,
        started.elapsed().as_secs_f32(),
        // The shaders compiled ahead, or none to compile: no bar.
        progress.share().unwrap_or(-1.0),
    ];
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
}
