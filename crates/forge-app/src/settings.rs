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
