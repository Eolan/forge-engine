//! Settings (D-069): the registry lives in [`forge_core::settings`], where every system can
//! register its own; this adds the command line's part, [`SettingsArgs`].

pub use forge_core::settings::*;

use std::path::PathBuf;

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
        let env = std::env::var_os("FORGE_CFG");
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
}
