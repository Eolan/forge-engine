//! Settings (D-069): the registry lives in [`forge_core::settings`], where every system can
//! register its own; this adds the command line's part, [`SettingsArgs`].

pub use forge_core::settings::*;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The command line's part of the settings: `--set NAME=VALUE` (repeatable) and the file.
/// A demo flattens it into its own arguments (`#[command(flatten)]`) and passes it to
/// [`crate::AppConfig::settings`].
#[derive(clap::Args, Clone, Debug, Default)]
pub struct SettingsArgs {
    /// Sets a console variable for the run (D-069): `--set r.display_info=2`. The console (the
    /// key under Esc) lists them: `list`, `find WORD`, `help NAME`.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    pub set: Vec<String>,
    /// The settings file (`name value` lines), read before `--set`: `forge.cfg` in the working
    /// directory by default, or `FORGE_CFG`.
    #[arg(long = "cfg", value_name = "PATH")]
    pub cfg: Option<PathBuf>,
    /// Reads no settings file (as `FORGE_CFG=none`): the scripts' runs, which the owner's own
    /// `forge.cfg` must not change.
    #[arg(long = "no-cfg")]
    pub no_cfg: bool,
    /// Runs a console line once the demo is ready, as if typed (repeatable): `--exec "list r."`;
    /// with `--set app.console=1`, a capture shows the console.
    #[arg(long = "exec", value_name = "LINE")]
    pub exec: Vec<String>,
}

impl SettingsArgs {
    /// The anti-aliasing the command line asks for (`--set r.aa=VALUE`, the last one), if any.
    pub fn asked_aa(&self) -> Option<&str> {
        self.set
            .iter()
            .filter_map(|s| split_assignment(s))
            .filter(|(name, _)| name.eq_ignore_ascii_case("r.aa"))
            .map(|(_, value)| value)
            .next_back()
    }

    /// Whether a run anti-aliases with DLSS where the device runs it (D-070): DLSS is the
    /// reference image, so scripted runs and their captures too, unless the run turns
    /// anti-aliasing off (`no_aa`, a demo's `--no-taa`, or `--set r.aa=off`) or asks for FSR
    /// (`--set r.aa=fsr`). DLAA's images are equal run to run since #277 and #281 (D-070's
    /// check), so the captures keep their 0 px.
    pub fn wants_dlss(&self, no_aa: bool) -> bool {
        !no_aa && !matches!(self.asked_aa(), Some("off" | "fsr"))
    }

    /// The settings file to read: `--cfg`, else `FORGE_CFG`, else `forge.cfg`; none with
    /// `--no-cfg` or `FORGE_CFG=none`.
    pub fn cfg_path(&self) -> Option<PathBuf> {
        self.cfg_path_from(std::env::var_os("FORGE_CFG"))
    }

    /// [`SettingsArgs::cfg_path`] with `FORGE_CFG` as given.
    fn cfg_path_from(&self, env: Option<OsString>) -> Option<PathBuf> {
        if self.no_cfg || env.as_ref().is_some_and(|v| v.is_empty() || v == "none") {
            return None;
        }
        Some(
            self.cfg
                .clone()
                .or_else(|| env.map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("forge.cfg")),
        )
    }

    /// The user's file (`user.cfg` in the settings folder `dir`: the menu's choices, the first
    /// launch's), read at start and written when the user changes a setting. None for a
    /// scripted run (`scripted`: a frame limit or a capture), whatever `FORGE_CFG` says, and
    /// none with `--no-cfg` or `FORGE_CFG=none` (#377, the owner: the tests use the test
    /// parameters, never what the user saved).
    pub fn user_file(&self, scripted: bool, dir: &Path) -> Option<PathBuf> {
        self.user_file_from(scripted, dir, std::env::var_os("FORGE_CFG"))
    }

    /// [`SettingsArgs::user_file`] with `FORGE_CFG` as given.
    fn user_file_from(&self, scripted: bool, dir: &Path, env: Option<OsString>) -> Option<PathBuf> {
        (!scripted && self.cfg_path_from(env).is_some()).then(|| dir.join("user.cfg"))
    }

    /// Reads a run's settings at start into `settings`, in their order: the user's file (`user`,
    /// [`SettingsArgs::user_file`]), `forge.cfg` (`cfg`, [`SettingsArgs::cfg_path`]), then
    /// `--set`; each applies as its variable registers. Returns the user's file's leading
    /// comment lines, kept when it is written again.
    pub fn load(&self, settings: &mut Settings, user: Option<&Path>, cfg: Option<&Path>) -> String {
        let mut header = String::new();
        if let Some(path) = user.filter(|p| p.exists()) {
            header = std::fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .take_while(|l| l.starts_with('#'))
                .map(|l| format!("{l}\n"))
                .collect();
            match settings.load_file_as(path, Source::User) {
                Ok(n) => tracing::info!(path = %path.display(), lines = n, "user settings read"),
                Err(e) => tracing::warn!("user settings: {e}"),
            }
        }
        if let Some(cfg) = cfg {
            match settings.load_file(cfg) {
                Ok(0) => {}
                Ok(n) => tracing::info!(path = %cfg.display(), lines = n, "settings file read"),
                Err(e) => tracing::warn!("settings file: {e}"),
            }
        }
        for assignment in &self.set {
            match split_assignment(assignment) {
                Some((name, value)) => {
                    if let Err(e) = settings.set_text(name, value, Source::CommandLine) {
                        tracing::warn!("--set {assignment}: {e}");
                    }
                }
                None => tracing::warn!("--set {assignment}: not NAME=VALUE"),
            }
        }
        header
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(set: &[&str]) -> SettingsArgs {
        SettingsArgs {
            set: set.iter().map(|s| (*s).to_owned()).collect(),
            ..SettingsArgs::default()
        }
    }

    #[test]
    fn dlss_unless_asked_otherwise() {
        // D-070: DLSS by default, scripted runs too.
        assert!(args(&[]).wants_dlss(false));
        assert!(args(&["r.aa=dlaa"]).wants_dlss(false));
        assert!(args(&["r.aa=dlss", "r.exposure=1"]).wants_dlss(false));
        // A demo's `--no-taa`, or the command line asking for none.
        assert!(!args(&[]).wants_dlss(true));
        assert!(!args(&["r.aa = off"]).wants_dlss(false));
        // FSR (D-070) in its place.
        assert!(!args(&["r.aa=fsr"]).wants_dlss(false));
        // The last `r.aa` counts, whatever its case.
        assert!(args(&["r.aa=fsr", "R.AA=dlaa"]).wants_dlss(false));
        assert_eq!(args(&["r.aa=off", "r.aa=fsr"]).asked_aa(), Some("fsr"));
    }

    /// The registry a demo would have, read from the files a run is allowed.
    fn started(args: &SettingsArgs, user: Option<&Path>, cfg: Option<&Path>) -> Settings {
        let mut s = Settings::new();
        args.load(&mut s, user, cfg);
        s.add(Spec::choice(
            "r.aa",
            &["off", "dlaa", "dlss", "fsr"],
            1,
            "aa",
        ));
        s.add(Spec::choice(
            "r.preset",
            &["custom", "low", "ultra"],
            0,
            "preset",
        ));
        s.add(Spec::choice(
            "app.window",
            &["windowed", "fullscreen"],
            0,
            "window",
        ));
        s.add(Spec::bool("r.ao", true, "ao"));
        s.add(Spec::float("r.brightness", 0.0, -2.0..=2.0, "brightness"));
        s
    }

    #[test]
    fn a_scripted_run_never_reads_the_users_file() {
        // #377: a user's saved choices that would change every image of a run.
        let dir = std::env::temp_dir().join(format!("forge-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("user.cfg"),
            "# poisoned\nr.aa off\nr.preset low\napp.window fullscreen\nr.ao 0\nr.brightness 2\n",
        )
        .unwrap();
        let cfg = dir.join("forge.cfg");
        std::fs::write(&cfg, "# empty\n").unwrap();
        let none = || Some(OsString::from("none"));
        let a = SettingsArgs::default();
        // A frame limit or a capture: no user file, whatever FORGE_CFG says.
        for env in [None, none(), Some(OsString::from(&cfg))] {
            assert_eq!(a.user_file_from(true, &dir, env), None);
        }
        // An interactive run reads it, unless the settings files are turned off.
        assert_eq!(
            a.user_file_from(false, &dir, None),
            Some(dir.join("user.cfg"))
        );
        assert_eq!(a.user_file_from(false, &dir, none()), None);
        let no_cfg = SettingsArgs {
            no_cfg: true,
            ..SettingsArgs::default()
        };
        assert_eq!(no_cfg.user_file_from(false, &dir, None), None);
        // So a scripted run's registry holds the defaults, as one with no user file at all...
        let user = a.user_file_from(true, &dir, None);
        let scripted = started(&a, user.as_deref(), Some(&cfg));
        let clean = started(&a, None, Some(&cfg));
        for s in [&scripted, &clean] {
            assert!(s.changed().is_empty(), "{:?}", s.changed());
            assert!(s.user_lines().is_empty());
        }
        // ...while an interactive run takes the user's choices.
        let user = a.user_file_from(false, &dir, None);
        let interactive = started(&a, user.as_deref(), Some(&cfg));
        assert_eq!(interactive.changed().len(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forge_cfg_and_the_command_line_override_the_users_file() {
        let dir = std::env::temp_dir().join(format!("forge-order-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let user = dir.join("user.cfg");
        std::fs::write(&user, "# first launch\nr.aa fsr\nr.ao 0\nr.preset low\n").unwrap();
        let cfg = dir.join("forge.cfg");
        std::fs::write(&cfg, "r.ao 1\n").unwrap();
        let a = args(&["r.preset=ultra"]);
        let mut s = Settings::new();
        assert_eq!(a.load(&mut s, Some(&user), Some(&cfg)), "# first launch\n");
        s.add(Spec::choice(
            "r.aa",
            &["off", "dlaa", "dlss", "fsr"],
            1,
            "aa",
        ));
        s.add(Spec::choice(
            "r.preset",
            &["custom", "low", "ultra"],
            0,
            "preset",
        ));
        s.add(Spec::bool("r.ao", false, "ao"));
        let value = |name: &str| s.info(name).map(|v| (v.value_text(), v.source));
        assert_eq!(value("r.aa"), Some(("fsr".to_owned(), Source::User)));
        assert_eq!(value("r.ao"), Some(("1".to_owned(), Source::File)));
        assert_eq!(
            value("r.preset"),
            Some(("ultra".to_owned(), Source::CommandLine))
        );
        // The user's file is written back as the user left it.
        assert_eq!(s.user_lines(), ["r.aa fsr", "r.ao 0", "r.preset low"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
