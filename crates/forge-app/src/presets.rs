//! Quality presets and the first launch's choices (the owner, 2026-10-25: "the first time
//! it's launched the engine should adapt itself to the computer it's on like modern game
//! engines do", and the player changes them "in the game parameters menu under graphics").
//!
//! - **A preset** (`r.preset`: custom, low, medium, high, ultra) is a patch over the renderer's
//!   shared settings ([`PATCHES`]): choosing one sets them, as from the user's file; changing one
//!   of them afterwards puts the preset back to custom. The names are the renderer's
//!   (`forge_render::settings`); a demo that registers none of a name leaves it pending, as a
//!   `--set` of an unknown name would.
//! - **The first launch** ([`first_launch`]): with no user file yet, the shell reads the GPU (its
//!   memory and whether it runs DLSS) and the monitor, picks a preset and the display, and writes
//!   them to the user's file (`settings/user.cfg`), which later runs read before `forge.cfg`.
//!   The menu's graphics page writes the same file.

/// `r.preset`'s choices.
pub const PRESETS: &[&str] = &["custom", "low", "medium", "high", "ultra"];
/// [`PRESETS`]' index of `custom`: no patch applied.
pub const CUSTOM: usize = 0;

/// What each preset sets, by the renderer's names: the resolution each upscaler renders at,
/// the ambient occlusion, the mirror rays, the local exposure and the clusters' error on screen.
/// `r.aa` is left as the device decided it (DLAA where DLSS runs, else FSR); the modes below it
/// make DLSS and FSR draw smaller alike, so either path takes the same step.
pub const PATCHES: &[(&str, &[(&str, &str)])] = &[
    (
        "low",
        &[
            ("r.dlss.mode", "performance"),
            ("r.fsr.mode", "performance"),
            ("r.ao", "0"),
            ("r.reflections.rays", "0"),
            ("r.exposure.local", "0"),
            ("r.lod.error", "3"),
        ],
    ),
    (
        "medium",
        &[
            ("r.dlss.mode", "balanced"),
            ("r.fsr.mode", "balanced"),
            ("r.ao", "1"),
            ("r.reflections.rays", "0"),
            ("r.exposure.local", "1"),
            ("r.lod.error", "2"),
        ],
    ),
    (
        "high",
        &[
            ("r.dlss.mode", "quality"),
            ("r.fsr.mode", "quality"),
            ("r.ao", "1"),
            ("r.reflections.rays", "1"),
            ("r.exposure.local", "1"),
            ("r.lod.error", "1"),
        ],
    ),
    (
        "ultra",
        &[
            ("r.dlss.mode", "quality"),
            ("r.fsr.mode", "native"),
            ("r.ao", "1"),
            ("r.reflections.rays", "1"),
            ("r.exposure.local", "1"),
            ("r.lod.error", "1"),
        ],
    ),
];

/// The patch of the preset named `preset`, empty for `custom` or an unknown name.
pub fn patch(preset: &str) -> &'static [(&'static str, &'static str)] {
    PATCHES
        .iter()
        .find(|(name, _)| *name == preset)
        .map_or(&[], |(_, patch)| patch)
}

/// Whether `name` is a setting some preset sets.
pub fn is_patched(name: &str) -> bool {
    PATCHES
        .iter()
        .any(|(_, patch)| patch.iter().any(|(n, _)| *n == name))
}

/// What the first launch found about the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Machine {
    /// The GPU's device-local memory, bytes (`None` when the device cannot say).
    pub video_memory: Option<u64>,
    /// Whether the device runs DLSS.
    pub dlss: bool,
    /// The window's monitor, pixels.
    pub monitor: Option<(u32, u32)>,
}

/// The preset the first launch picks for `machine`, by its video memory: the engine's target
/// (an RTX 5070 Ti, 16 GB) and above take ultra, 8 GB high, 5 GB medium, less low; a device
/// that cannot say its memory takes medium.
pub fn preset_for(machine: &Machine) -> &'static str {
    const GIB: u64 = 1 << 30;
    match machine.video_memory {
        Some(bytes) if bytes >= 12 * GIB => "ultra",
        Some(bytes) if bytes >= 8 * GIB => "high",
        Some(bytes) if bytes >= 5 * GIB => "medium",
        Some(_) => "low",
        None => "medium",
    }
}

/// The first launch's settings for `machine`, as `name value` lines of the user's file: the
/// preset, and borderless fullscreen on the monitor (its own resolution, which the upscaler then
/// renders under by the preset's mode).
pub fn first_launch(machine: &Machine) -> Vec<(String, String)> {
    vec![
        ("r.preset".to_owned(), preset_for(machine).to_owned()),
        ("app.window".to_owned(), "fullscreen".to_owned()),
    ]
}

/// The header of a user file the first launch wrote: what it saw.
pub fn first_launch_header(machine: &Machine, gpu: &str) -> String {
    let memory = machine.video_memory.map_or("unknown".to_owned(), |b| {
        format!("{:.1} GiB", b as f64 / f64::from(1u32 << 30))
    });
    let monitor = machine
        .monitor
        .map_or("unknown".to_owned(), |(w, h)| format!("{w} x {h}"));
    format!(
        "# Written at the first launch: {gpu}, {memory} of video memory, DLSS {}, monitor {monitor}.\n\
         # The game's menu (Esc, Settings, Graphics) rewrites this file; `forge.cfg` and --set override it.",
        if machine.dlss { "yes" } else { "no" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preset_follows_the_video_memory() {
        let machine = |gib: Option<u64>| Machine {
            video_memory: gib.map(|g| g << 30),
            dlss: true,
            monitor: Some((2560, 1440)),
        };
        assert_eq!(preset_for(&machine(Some(16))), "ultra");
        assert_eq!(preset_for(&machine(Some(12))), "ultra");
        assert_eq!(preset_for(&machine(Some(8))), "high");
        assert_eq!(preset_for(&machine(Some(6))), "medium");
        assert_eq!(preset_for(&machine(Some(4))), "low");
        assert_eq!(preset_for(&machine(None)), "medium");
        let lines = first_launch(&machine(Some(8)));
        assert_eq!(lines[0], ("r.preset".to_owned(), "high".to_owned()));
        assert!(first_launch_header(&machine(Some(8)), "RTX").contains("8.0 GiB"));
    }

    #[test]
    fn every_preset_patches_the_same_names_and_custom_none() {
        let names = |p: &str| {
            let mut n: Vec<&str> = patch(p).iter().map(|(n, _)| *n).collect();
            n.sort_unstable();
            n
        };
        for preset in &PRESETS[1..] {
            assert_eq!(names(preset), names("low"), "{preset}");
            assert!(patch(preset).iter().all(|(n, _)| is_patched(n)));
        }
        assert!(patch("custom").is_empty());
        assert!(patch("nothing").is_empty());
        assert!(!is_patched("r.aa"));
        assert_eq!(PRESETS[CUSTOM], "custom");
    }
}
