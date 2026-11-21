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
//!   memory and whether it has ray queries) and the monitor, picks a preset, the display and, by
//!   the preset, whether the frame is traced (D-111), and writes them to the user's file
//!   (`settings/user.cfg`), which later runs read before `forge.cfg`. The menu's graphics page
//!   writes the same file.

use forge_core::settings::{Settings, Source, Var};

/// `r.preset`'s choices.
pub const PRESETS: &[&str] = &["custom", "low", "medium", "high", "ultra"];
/// [`PRESETS`]' index of `custom`: no patch applied.
pub const CUSTOM: usize = 0;

/// What each preset sets, by the renderer's names: the resolution each upscaler renders at,
/// the ambient occlusion, the mirror rays, the local exposure and the clusters' error on screen.
/// `r.aa` keeps the family the device or the player chose (NVIDIA's where DLSS runs, else FSR),
/// and within NVIDIA's a preset picks DLAA or DLSS ([`patch_at`]): `r.dlss.mode` counts under
/// `dlss` alone, so that DLSS and FSR draw smaller alike and either path takes the same step.
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

/// `r.aa`'s choice for `preset` within NVIDIA's family: `dlaa` for ultra, the image at the
/// window's size, and `dlss` for the others, which render smaller by their `r.dlss.mode`. `None`
/// for custom or an unknown name. Until 2026-11-21 a preset left DLAA as it was, where
/// `r.dlss.mode` counts for nothing: on an RTX GPU low, medium and high drew at the window's
/// size (the village's street on an RTX 5060 Laptop GPU at 1600 x 900: 19.5 ms a frame with
/// low, 20.4 with none, 14.0 once it upscaled).
pub fn dlss_family(preset: &str) -> Option<&'static str> {
    match preset {
        "ultra" => Some("dlaa"),
        "low" | "medium" | "high" => Some("dlss"),
        _ => None,
    }
}

/// What `preset` sets with the anti-aliasing at `aa` (`r.aa` as it stands, `None` for a demo
/// that renders no smaller under DLSS: it registers no `r.dlss.mode`): its patch and, where `aa`
/// is NVIDIA's (`dlaa` or `dlss`), which of the two ([`dlss_family`]). Under FSR, whose own mode
/// is in the patch, or with no anti-aliasing, `r.aa` stays: a preset changes no family.
pub fn patch_at(preset: &str, aa: Option<&str>) -> Vec<(&'static str, &'static str)> {
    let mut patch = patch(preset).to_vec();
    if matches!(aa, Some("dlaa" | "dlss"))
        && let Some(family) = dlss_family(preset)
    {
        patch.push(("r.aa", family));
    }
    patch
}

/// Whether a preset picks DLAA or DLSS at all ([`patch_at`]): not one read from the user's file
/// (`from_users_file`) beside an `r.aa` the file holds too (`file_holds_aa`). That line is the
/// player's choice, or a preset's own and then already its pick: a file written before presets
/// picked may say high and DLAA, and keeps the image the player chose.
pub fn picks_family(from_users_file: bool, file_holds_aa: bool) -> bool {
    !(from_users_file && file_holds_aa)
}

/// Whether `r.aa` at `aa` is as `preset` leaves it ([`patch_at`]): any value outside NVIDIA's
/// family, and the preset's own choice within it.
pub fn aa_kept(preset: &str, aa: &str) -> bool {
    !matches!(aa, "dlaa" | "dlss") || dlss_family(preset).is_none_or(|family| family == aa)
}

/// What following `r.preset` did on a frame ([`Follower::follow`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Followed {
    /// Nothing to do.
    Nothing,
    /// `preset`'s patch was set: `chosen` when the preset itself changed, not when settings
    /// registered under a preset already in force.
    Applied {
        /// The preset in force.
        preset: &'static str,
        /// Whether `r.preset` changed, or settings registered under it.
        chosen: bool,
    },
    /// A setting the preset sets was changed by hand: `r.preset` is custom again.
    Custom {
        /// The preset it was.
        was: &'static str,
        /// The setting changed.
        by: String,
    },
}

/// `r.preset` followed from frame to frame. A preset chosen sets its patch ([`patch_at`]), as
/// from where the preset came (the user's file, the menu, the console, `--set`); a setting it
/// sets changed by hand since puts the preset back to custom. A preset in force is applied again
/// when settings register (a demo's own, after the first frames): its values that waited for
/// them land then, which is no hand's change, and its choice of DLAA or DLSS needs `r.aa` to
/// stand. Until 2026-11-21 neither held: a preset chosen at start read as custom once the demo
/// had registered, and drew no smaller under DLSS.
#[derive(Clone, Copy, Debug, Default)]
pub struct Follower {
    /// The settings' change count when the preset was last applied.
    applied: u64,
    /// How many settings were registered then.
    vars: usize,
}

impl Follower {
    /// Follows `preset` (`r.preset`) in `s` for a frame.
    pub fn follow(&mut self, s: &mut Settings, preset: Var<usize>) -> Followed {
        let index = s.get(preset);
        let name = PRESETS.get(index).copied().unwrap_or(PRESETS[CUSTOM]);
        let registered = s.vars().count();
        let grown = std::mem::replace(&mut self.vars, registered) != registered;
        let chosen = s.changed_since(preset, self.applied);
        if chosen || (grown && index != CUSTOM) {
            if index != CUSTOM {
                let source = s.source(preset);
                // Where the demo renders smaller under DLSS, the preset picks DLAA or DLSS too,
                // unless it comes from the user's file beside an `r.aa` of the file's own.
                let picks = picks_family(source == Source::User, s.user_value("r.aa").is_some());
                let aa = s
                    .info("r.dlss.mode")
                    .filter(|_| picks)
                    .and_then(|_| s.info("r.aa"))
                    .map(|v| v.value_text());
                for (setting, value) in patch_at(name, aa.as_deref()) {
                    if let Err(e) = s.set_text(setting, value, source) {
                        tracing::warn!("r.preset {name}: {setting}: {e}");
                    }
                }
            }
            self.applied = s.changes();
            return match index {
                CUSTOM => Followed::Nothing,
                _ => Followed::Applied {
                    preset: name,
                    chosen,
                },
            };
        }
        if index == CUSTOM {
            return Followed::Nothing;
        }
        // A setting the preset sets, changed since: the hand's, not the preset's.
        let by = s
            .vars()
            .find(|v| {
                v.changed > self.applied
                    && (is_patched(&v.name)
                        || (v.name == "r.aa" && !aa_kept(name, &v.value_text())))
            })
            .map(|v| v.name.clone());
        match by {
            Some(by) => {
                let source = s.source(preset);
                s.set_by(preset, CUSTOM, source);
                self.applied = s.changes();
                Followed::Custom { was: name, by }
            }
            None => Followed::Nothing,
        }
    }
}

/// What the first launch found about the machine, before a device is made of its GPU: what its
/// choices follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Machine {
    /// The GPU's device-local memory, bytes (`None` when the GPU cannot say).
    pub video_memory: Option<u64>,
    /// Whether the GPU has ray queries: only there is the tier a choice (D-108).
    pub ray_query: bool,
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

/// Whether the first launch keeps the rays where it picked `preset`, on a GPU that has ray
/// queries (D-111, the owner, 2026-11-21: by the preset): high and ultra keep them; low and
/// medium (under 8 GiB of video memory, or a GPU that cannot say) take the tier without rays
/// (D-108), which the menu's "Ray tracing" row switches back. The presets themselves never set
/// `r.rays` ([`PATCHES`]): choosing one in the menu later changes no tier.
pub fn first_launch_keeps_rays(preset: &str) -> bool {
    matches!(preset, "high" | "ultra")
}

/// The first launch's settings for `machine`, as `name value` lines of the user's file: the
/// preset, borderless fullscreen on the monitor (its own resolution, which the upscaler then
/// renders under by the preset's mode), and `r.rays 0` where the GPU has ray queries and the
/// preset does not keep them ([`first_launch_keeps_rays`]). Nothing about rays on a GPU without
/// ray queries: the file would hold a choice that GPU never had.
pub fn first_launch(machine: &Machine) -> Vec<(String, String)> {
    let preset = preset_for(machine);
    let mut lines = vec![
        ("r.preset".to_owned(), preset.to_owned()),
        ("app.window".to_owned(), "fullscreen".to_owned()),
    ];
    if machine.ray_query && !first_launch_keeps_rays(preset) {
        lines.push(("r.rays".to_owned(), "0".to_owned()));
    }
    lines
}

/// The header of a user file the first launch wrote: what it saw, `dlss` whether the device made
/// of the GPU runs DLSS.
pub fn first_launch_header(machine: &Machine, gpu: &str, dlss: bool) -> String {
    let memory = machine.video_memory.map_or("unknown".to_owned(), |b| {
        format!("{:.1} GiB", b as f64 / f64::from(1u32 << 30))
    });
    let monitor = machine
        .monitor
        .map_or("unknown".to_owned(), |(w, h)| format!("{w} x {h}"));
    let yes = |b: bool| if b { "yes" } else { "no" };
    format!(
        "# Written at the first launch: {gpu}, {memory} of video memory, ray queries {}, DLSS {}, monitor {monitor}.\n\
         # The game's menu (Esc, Settings, Graphics) rewrites this file; `forge.cfg` and --set override it.",
        yes(machine.ray_query),
        yes(dlss)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preset_follows_the_video_memory() {
        let machine = |gib: Option<u64>| Machine {
            video_memory: gib.map(|g| g << 30),
            ray_query: true,
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
        let header = first_launch_header(&machine(Some(8)), "RTX", true);
        assert!(header.contains("8.0 GiB"));
        assert!(header.contains("ray queries yes, DLSS yes"));
    }

    /// D-111: on a GPU with ray queries the first launch takes the tier without rays where it
    /// picks the low or the medium preset, and keeps the rays with high and ultra; on a GPU
    /// without ray queries it writes nothing about them; and no preset sets `r.rays`.
    #[test]
    fn the_first_launch_picks_the_tier_by_the_preset() {
        let machine = |gib: Option<u64>, ray_query: bool| Machine {
            video_memory: gib.map(|g| g << 30),
            ray_query,
            monitor: None,
        };
        let rays = |m: Machine| {
            first_launch(&m)
                .into_iter()
                .find(|(name, _)| name == "r.rays")
                .map(|(_, value)| value)
        };
        for (gib, preset, line) in [
            (Some(16), "ultra", None),
            (Some(8), "high", None),
            (Some(7), "medium", Some("0")),
            (Some(4), "low", Some("0")),
            (None, "medium", Some("0")),
        ] {
            assert_eq!(preset_for(&machine(gib, true)), preset);
            assert_eq!(rays(machine(gib, true)).as_deref(), line, "{preset}");
            assert_eq!(rays(machine(gib, false)), None, "{preset}, no ray queries");
            assert_eq!(first_launch_keeps_rays(preset), line.is_none());
        }
        assert!(!is_patched("r.rays"));
        assert!(
            first_launch_header(&machine(Some(7), false), "GPU", false)
                .contains("ray queries no, DLSS no")
        );
    }

    /// The first launch's lines as the shell applies them, before it makes the device: `r.rays`
    /// reads 0 for the device where nothing above the user's file set it, the file holds the
    /// line either way, and `--set r.rays=1` keeps this run traced.
    #[test]
    fn the_first_launchs_tier_is_in_force_before_the_device_and_under_the_command_line() {
        use forge_core::settings::{Settings, Source, Spec};
        let laptop = Machine {
            video_memory: Some(7131 << 20),
            ray_query: true,
            monitor: None,
        };
        for by_command_line in [false, true] {
            let mut s = Settings::new();
            let rays = s.add(Spec::bool("r.rays", true, "rays"));
            if by_command_line {
                s.set_text("r.rays", "1", Source::CommandLine).unwrap();
            }
            for (name, value) in first_launch(&laptop) {
                s.set_user_beneath(&name, &value).unwrap();
            }
            assert_eq!(s.get(rays), by_command_line);
            assert_eq!(
                s.user_lines(),
                ["app.window fullscreen", "r.preset medium", "r.rays 0"]
            );
        }
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

    /// Within NVIDIA's family a preset picks DLAA or DLSS, so that its `r.dlss.mode` counts;
    /// under FSR, with no anti-aliasing, or in a demo that renders no smaller, `r.aa` stays.
    #[test]
    fn a_preset_picks_dlaa_or_dlss_within_nvidias_family() {
        let aa = |preset: &str, at: Option<&str>| {
            patch_at(preset, at)
                .into_iter()
                .find(|(name, _)| *name == "r.aa")
                .map(|(_, value)| value)
        };
        for at in ["dlaa", "dlss"] {
            assert_eq!(aa("ultra", Some(at)), Some("dlaa"));
            for preset in ["low", "medium", "high"] {
                assert_eq!(aa(preset, Some(at)), Some("dlss"), "{preset} from {at}");
            }
            assert_eq!(aa("custom", Some(at)), None);
        }
        for preset in &PRESETS[1..] {
            for at in [Some("fsr"), Some("off"), None] {
                assert_eq!(aa(preset, at), None, "{preset} at {at:?}");
                assert_eq!(patch_at(preset, at), patch(preset));
            }
            // The rest of the patch is the preset's, whatever the family.
            assert_eq!(
                patch_at(preset, Some("dlaa"))[..patch(preset).len()],
                *patch(preset)
            );
        }
        // A hand's change of `r.aa` keeps the preset outside the family or at its own choice.
        assert!(aa_kept("low", "fsr") && aa_kept("low", "off") && aa_kept("low", "dlss"));
        assert!(!aa_kept("low", "dlaa") && !aa_kept("ultra", "dlss") && aa_kept("ultra", "dlaa"));
        assert!(aa_kept("custom", "dlaa") && aa_kept("custom", "dlss"));
        // A preset from the user's file leaves the file's own `r.aa`; from anywhere else, or
        // with no such line (the first launch's file), it picks.
        assert!(!picks_family(true, true));
        assert!(
            picks_family(true, false) && picks_family(false, true) && picks_family(false, false)
        );
    }

    /// The settings a demo that renders smaller registers after the shell's first frames.
    fn register_a_demo(s: &mut Settings, upscales: bool) {
        use forge_core::settings::Spec;
        s.add(Spec::choice(
            "r.aa",
            &["off", "dlaa", "dlss", "fsr"],
            1,
            "aa",
        ));
        if upscales {
            s.add(Spec::choice(
                "r.dlss.mode",
                &["quality", "balanced", "performance", "ultra-performance"],
                0,
                "dlss",
            ));
            s.add(Spec::choice(
                "r.fsr.mode",
                &[
                    "native",
                    "quality",
                    "balanced",
                    "performance",
                    "ultra-performance",
                ],
                0,
                "fsr",
            ));
        }
        s.add(Spec::bool("r.ao", true, "ao"));
        s.add(Spec::bool("r.reflections.rays", true, "rays"));
        s.add(Spec::bool("r.exposure.local", true, "local"));
        s.add(Spec::float("r.lod.error", 1.0, 0.125..=16.0, "lod"));
    }

    fn text(s: &Settings, name: &str) -> String {
        s.info(name).map(|v| v.value_text()).unwrap_or_default()
    }

    /// The order a run has: the preset is chosen and followed on the first frame, the demo
    /// registers its settings later. The preset keeps its name, its values land, and under
    /// NVIDIA's anti-aliasing it draws smaller; then a hand's change makes it custom.
    #[test]
    fn a_preset_chosen_before_the_demo_registers_holds_and_lands() {
        use forge_core::settings::Spec;
        let mut s = Settings::new();
        let preset = s.add(Spec::choice("r.preset", PRESETS, 0, "preset"));
        let mut follower = Follower::default();
        assert_eq!(follower.follow(&mut s, preset), Followed::Nothing);
        s.set_text("r.preset", "low", Source::CommandLine).unwrap();
        assert_eq!(
            follower.follow(&mut s, preset),
            Followed::Applied {
                preset: "low",
                chosen: true
            }
        );
        // The demo's settings register: the values that waited land, and DLSS is picked.
        register_a_demo(&mut s, true);
        assert_eq!(
            follower.follow(&mut s, preset),
            Followed::Applied {
                preset: "low",
                chosen: false
            }
        );
        assert_eq!(text(&s, "r.preset"), "low");
        assert_eq!(text(&s, "r.ao"), "0");
        assert_eq!(text(&s, "r.aa"), "dlss");
        assert_eq!(text(&s, "r.dlss.mode"), "performance");
        assert_eq!(follower.follow(&mut s, preset), Followed::Nothing);
        // FSR by hand keeps the preset: it changes no family, and FSR's mode is the preset's.
        s.set_text("r.aa", "fsr", Source::Console).unwrap();
        assert_eq!(follower.follow(&mut s, preset), Followed::Nothing);
        // DLAA by hand is not low's, and a setting of the patch by hand is not either.
        s.set_text("r.aa", "dlaa", Source::Console).unwrap();
        assert_eq!(
            follower.follow(&mut s, preset),
            Followed::Custom {
                was: "low",
                by: "r.aa".to_owned()
            }
        );
        assert_eq!(text(&s, "r.preset"), "custom");
        s.set_text("r.preset", "ultra", Source::Console).unwrap();
        follower.follow(&mut s, preset);
        assert_eq!(text(&s, "r.aa"), "dlaa");
        s.set_text("r.ao", "0", Source::Console).unwrap();
        assert_eq!(
            follower.follow(&mut s, preset),
            Followed::Custom {
                was: "ultra",
                by: "r.ao".to_owned()
            }
        );
        assert_eq!(follower.follow(&mut s, preset), Followed::Nothing);
    }

    /// A preset read from the user's file leaves the file's own `r.aa`; the first launch's
    /// file has none, and its preset picks; a demo that renders no smaller keeps DLAA.
    #[test]
    fn a_preset_from_the_users_file_leaves_the_files_own_anti_aliasing() {
        use forge_core::settings::Spec;
        let run = |file_aa: Option<&str>, upscales: bool| {
            let mut s = Settings::new();
            let preset = s.add(Spec::choice("r.preset", PRESETS, 0, "preset"));
            s.set_text("r.preset", "high", Source::User).unwrap();
            if let Some(aa) = file_aa {
                s.set_text("r.aa", aa, Source::User).unwrap();
            }
            let mut follower = Follower::default();
            follower.follow(&mut s, preset);
            register_a_demo(&mut s, upscales);
            follower.follow(&mut s, preset);
            assert_eq!(follower.follow(&mut s, preset), Followed::Nothing);
            assert_eq!(text(&s, "r.preset"), "high");
            text(&s, "r.aa")
        };
        assert_eq!(
            run(Some("dlaa"), true),
            "dlaa",
            "the player's DLAA under high"
        );
        assert_eq!(run(None, true), "dlss", "the first launch's file");
        assert_eq!(run(None, false), "dlaa", "a demo that renders no smaller");
    }
}
