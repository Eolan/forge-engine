# Research — A developer console and console variables (D-069)

> Written 2026-10-17 for D-069 (#278). The owner asked on 2026-10-17:
>
> > "Could we backport the changes over the demos and make it overall consistent? And make a in engine dev console activable with the key ² and with autocompletion like most of the game engines do. And use the flags only when we do something outside of base behaviour for now?"
>
> The owner's decisions for D-069, which this note takes as given:
> 1. A registry of typed console variables that the engine's systems and each demo register. Each has a dotted name (`r.aa`, `r.display_info`, `app.fixed_step`), a default, a range, a help line and flags.
> 2. Any variable can be set on the command line, and in a hand-edited `forge.cfg`. Console changes are temporary: nothing is saved.
> 3. An in-game console on the physical key under Esc (² on AZERTY, ` on QWERTY), with Tab completion, history, `help`, `find` and `list`.
> 4. Release builds keep the console, limited to information-only variables (like Star Citizen's `r_displayInfo`) and the build's version. Nothing in it may give a player an advantage.
>
> **How facts were checked.** Every source was looked up on 2026-10-17 with WebFetch or WebSearch only. Nothing was downloaded, cloned or opened in a browser. Each source is marked:
> - *read*: the page or file was fetched and read;
> - *search-recorded*: seen only in a search engine's summary, because the host refused the fetch (HTTP 403), the file was not at the fetched path, or the page's text was only navigation.
>
> Facts marked *from memory* say so where they are used. Patents were not searched, as the owner asked. The Quake III, Doom 3 and Source SDK files are GPL or Valve-licensed. They were read to learn their designs, and Forge copies no code from them.
>
> Forge's own facts were read locally on 2026-10-17 from:
> - `docs/DECISIONS.md` (D-069);
> - `crates/forge-app/src/lib.rs` (the keyboard handler);
> - `crates/forge-app/src/overlay.rs` (95 printable ASCII glyphs, codes 32 to 126);
> - `crates/forge-app/src/profile.rs` (the F1 overlay's Off, Compact and Full modes);
> - `crates/forge-app/src/input.rs`;
> - the demos' `Cargo.toml` files (each demo parses its flags with clap).

---

## The short answer

- **Every engine uses the same model.** A console variable has a name, a typed value, a default, a help line and flags. It is set by typing `name value`, read by typing `name`, and set before start-up from the command line or a config file. This holds for Quake III [1][2][3], Doom 3 [8], Source [10][11], Unreal [18][19], CryEngine [26][27] and O3DE [33][34]. The models differ in five places:
  - **Typing.** Quake III stores strings and parses them with `atof`/`atoi` on every set [1][2]. Doom 3 adds type flags (bool, integer, float), a min/max range and a list of allowed strings [8]. Unreal, O3DE and the Rust `cvars` crate use real types [18][34][41].
  - **Ranges.** Source clamps an out-of-range value to the nearest bound without saying so [12]. Doom 3 and the `cvar` crate's `ClampedProp` also bound values [8][42].
  - **Change detection.** Quake III keeps a change counter that modules poll [2]. Unreal's documentation prefers polling and warns that change callbacks can deadlock [18]. Source fires a callback only when the string really changed [12]. CryEngine sinks can veto a change [29].
  - **Restart-only values.** Quake III latches the new value until the code next registers the variable [1][2]. CryEngine flags such variables as needing an app restart or a level reload [26], and O3DE has `NeedsReload` [33].
  - **Development-only variables.** Source hides `FCVAR_DEVELOPMENTONLY` variables from completion in release builds [10][13]. CryEngine does not even register `VF_DEV_ONLY` variables outside development builds [26]. Doom 3 treats every variable as a cheat unless a flag says otherwise [8]. Unreal removes the console from Shipping builds altogether [22][23].
- **Persistence is where engines most often go wrong for Forge's purpose.** Quake III rewrites `q3config.cfg` whenever an archived variable changes [3]. Source and CryEngine also keep saved flags [10][26]. The owner ruled this out: `forge.cfg` is written only by hand, and console changes end with the run. Unreal's `ConsoleVariables.ini` is the nearest model: a local file for the developer, read at start-up [18].
- **Completion.** Quake III and Doom 3 complete to the longest common prefix and print the candidates [3][9]. Doom 3 then cycles through them on each further Tab and prints each variable's value [9]. Source shows up to 10 candidates with their values while you type [13]. Unreal can show the first line of help beside each candidate [20]. Doom 3 and Source also complete a variable's or command's arguments [8][9][11][13]. Forge should combine these: complete to the common prefix, then cycle; show value and help beside each candidate; complete choices after the name.
- **Release builds.** The safe pattern is to leave dev variables out of shipping builds altogether, not to hide them or switch them off at run time:
  - Source's hidden variables can be revealed by a plugin [16].
  - CryEngine's restricted console mode is a variable, `con_restricted` [28]. Star Citizen's community wiki says `Con_restricted = 0` in `USER.cfg` "allows for more commands" [46].
  - Quake III's cheat gate needed fixes for loopholes and startup order [2][7].
  - Forge should compile the gate as a cargo feature (`shipping`). In a shipping build, a variable without the `RELEASE` flag has no name, cannot be set, and reads as its default. That default is also the shipped look under D-069.
- **The toggle key with winit 0.30.**
  - Match `PhysicalKey::Code(KeyCode::Backquote)`. winit maps Windows scancode 0x29 to `Backquote` [57][60]. That is the top-left key under Esc on both ANSI and ISO keyboards. On a French Windows layout it types ², on a US layout ` [61].
  - Since winit 0.29 the typed text arrives inside the same `KeyEvent` as `text` [52][58]. Swallowing the toggle key's event therefore keeps ² out of the line.
  - Unreal matches console keys by key name, and its ² problem was closed as Won't Fix (UE-15894) [24]. ioquake3 had to add a separate console-key variable and a Shift-Esc toggle for non-US layouts [4][5]. Matching the physical key avoids both problems.

| | Typed values | Range | Change detection | Restart-only | Saved | Dev-only in release |
|---|---|---|---|---|---|---|
| Quake III [1][2][3] | strings, parsed on set | no | `modificationCount` (poll) | `CVAR_LATCH`, `CVAR_INIT` | `CVAR_ARCHIVE`, auto-written | `CVAR_CHEAT` + `sv_cheats` (run time) |
| Doom 3 [8][9] | type flags | min/max, string list | `CVAR_MODIFIED` (poll) | `CVAR_INIT` | `CVAR_ARCHIVE` | everything is a cheat unless flagged |
| Source [10][11][12] | float/int/string | min/max, clamped | per-variable + global callbacks | reload flags | `FCVAR_ARCHIVE` | `FCVAR_DEVELOPMENTONLY` hidden; `FCVAR_CHEAT` + `sv_cheats` |
| Unreal [18][19] | int/float/bool/string | no | polling preferred; sinks; callbacks discouraged | none | ini files; `SaveForNextBoot` | `ECVF_Cheat`; no console in Shipping |
| CryEngine [26][27] | int/float/string | no | sinks with veto | app restart, level reload | `VF_DUMPTODISK` (meaning not documented) | `VF_DEV_ONLY` not registered; `VF_CHEAT`; `con_restricted` |
| O3DE [33][34] | real types, custom types | no | callback (any thread) | `NeedsReload` | `.cfg`, `.setreg` | `IsCheat`; "basic access protections" |
| `cvars` crate [41] | struct fields | no | none | no | load/save files | no |

---

## 1. id Tech: Quake III Arena and Doom 3

### 1.1 Quake III's `cvar_t` and its flags

- **Storage.** A `cvar_t` holds `name`, `string`, `resetString` (the default), `latchedString`, `flags`, `modified`, `modificationCount`, `value` (from `atof`) and `integer` (from `atoi`) [1]. Every value is a string, parsed again on each set [2].
- **Flags** [1]:
  - `CVAR_ARCHIVE`: "set to cause it to be saved".
  - `CVAR_USERINFO`, `CVAR_SERVERINFO`, `CVAR_SYSTEMINFO`: network replication.
  - `CVAR_INIT`: "don't allow change from console at all, but can be set from the command line".
  - `CVAR_LATCH`: "will only change when C code next does a Cvar_Get()".
  - `CVAR_ROM`: "display only, cannot be set by user at all".
  - `CVAR_USER_CREATED`: "created by a set command".
  - `CVAR_TEMP`: "can be set even when cheats are disabled, but is not archived".
  - `CVAR_CHEAT`: "can not be changed if cheats are disabled".
  - `CVAR_NORESTART`: "do not clear when a cvar_restart is issued".
- **Registration.** `Cvar_Get` creates the variable, or returns the existing one and ORs the new flags into it. Flags only accumulate. A second, different default only prints a warning in developer mode. If the user created the variable earlier with `set`, the code's registration adopts it and clears `CVAR_USER_CREATED` [2].
- **Setting.** `Cvar_Set2` without `force` (the console's path) checks the flags in order [2]:
  1. ROM refuses: "is read only".
  2. INIT refuses: "is write protected".
  3. LATCH stores `latchedString` and prints "will be changed upon restarting".
  4. CHEAT refuses unless `sv_cheats` is non-zero.
  - Code sets with `force` and skips every check.
- **Change detection.** Polling. Each real change sets `modified` and increments `modificationCount`. The game modules hold a copy (`vmCvar_t`) and refresh it only when the counts differ [1][2].
- **Commands** [2]:
  - `set`, `seta` (set and archive), `sets` and `setu` (set and replicate);
  - `toggle` ("for easy single key binding");
  - `reset` (back to `resetString`);
  - `cvarlist [filter]`, which prints flag letters (S, U, R, I, A, L, C) before each name and value;
  - `cvar_restart`.
  - Typing a name alone prints its value, its default and any latched value.
- **Cheats.** `sv_cheats` is a ROM, server-replicated variable [2]. `Cvar_SetCheatState` resets every cheat variable to its default when cheats go off. Its own comment says latched cheat variables may escape that reset [2]. ioquake3 bug 4974 shows how fragile the gate was. A reset added against an earlier cheat loophole also wiped cheat variables set on purpose with `+set` at start-up. It was moved after review [7].

### 1.2 Quake III's command line, config files and saving

- **Command line.** The command line is split into console lines at each `+` [3]. `Com_StartupVariable` applies the `set` lines twice:
  - before the config files run, so that paths and `developer` take effect early;
  - after them, to "override anything from the config files with command line args" [3].
  - Other `+` lines are queued as console commands [3].
- **Config order** [3]:
  1. `default.cfg`;
  2. `q3config.cfg` (skipped in safe mode, `+set safe` or `cvar_restart` on the command line);
  3. `autoexec.cfg`.
- **Saving.** Every frame, if any archived variable changed, `Com_WriteConfiguration` rewrites `q3config.cfg` with `seta name "value"` lines. It writes a latched value in place of the current one [2][3]. The user's typed changes are therefore saved silently.
- **`developer`.** It is a `CVAR_TEMP` variable that gates `Com_DPrintf` [3].

### 1.3 Quake III's Tab completion

- `Field_CompleteCommand` completes only the first word of the line, case-insensitively, against both commands and variables [3].
- **One match:** the line becomes that name followed by a space [3].
- **Several matches:** the line becomes their longest common prefix, and the names are printed below. Values are not shown [3].
- Arguments and values are not completed in this version [3].

### 1.4 The console key on non-US layouts (ioquake3)

- In 2007 Quake III opened the console only on `` ` `` or `~`. Binding another key failed while the menus had keyboard focus. A Shift-Esc toggle was proposed because "All users currently need to be able to type '`' or '~'" [5].
- In 2008 ioquake3 added `cl_consoleKeys`, a list of key names or character codes, defaulting to `~ ` 0x7e 0x60` [4]. Its code skips keys "that have ASCII names but produce no character", which are probably dead keys [4].
- Bug 4784 (2010) reports that ioquake3 ignored the backquote key because it produced no character. The patch handled console keys "before dead keys get droped" [6].
- Lesson: matching the console key by character breaks on layouts. A physical key avoids it.

### 1.5 Doom 3 (id Tech 4): typed variables and value completion

- **Declaration.** `idCVar` objects are declared statically where they are used. They are collected in a static list and registered when the system starts. "CVars may be declared multiple times using the same name string", and all copies share one value [8].
- **Types and ranges.** "CVars should always be declared with the correct type flag: CVAR_BOOL, CVAR_INTEGER or CVAR_FLOAT" [8]. The three constructors take [8]:
  - an optional value-completion callback;
  - a `valueMin`/`valueMax` range;
  - or a list of allowed strings (`valueStrings`).
  - Boolean variables get `ArgCompletion_Boolean` automatically [8].
- **Cheats by default.** "CVars are always considered cheats except when CVAR_NOCHEAT, CVAR_INIT, CVAR_ROM, CVAR_ARCHIVE, CVAR_USERINFO, CVAR_SERVERINFO, CVAR_NETWORKSYNC is set" [8]. A forgotten flag therefore makes a variable a cheat, not a public setting.
- **Completion** (`idEditField::AutoComplete`) [9]:
  - The first Tab completes to the longest common prefix of commands and variables.
  - It prints every candidate. Variables are printed with their current value: `name = "value"`.
  - Each further Tab replaces the line with the next candidate, wrapping around.
  - With one match, or once arguments are typed, it calls the command's or variable's argument completion instead. That covers allowed strings, booleans and file names.

---

## 2. Source (Valve)

- **Declaration.** `ConVar` and `ConCommand` objects are declared where they are used, with a name, a default, flags and a help string [11]. Overloads add [11]:
  - a float minimum and maximum (`bMin, fMin, bMax, fMax`);
  - a change callback, `void (*)(IConVar *var, const char *pOldValue, float flOldValue)` [10];
  - "competitive" bounds.
- **Argument completion.** A `ConCommand` can take a completion callback. It fills at most 64 items of 64 characters each (`COMMAND_COMPLETION_MAXITEMS`, `COMMAND_COMPLETION_ITEM_LENGTH`) [11].
- **Reading.** `GetFloat`, `GetInt` and `GetBool` read a value directly. `ConVarRef` reads or writes a variable by name [11].
- **Ranges.** `ClampValue` snaps a value outside the range to the nearest bound and rewrites the string. The change is not refused [12].
- **Callbacks.**
  - `ChangeStringValue` runs callbacks only when the string really changed ("If nothing has changed, don't do the callbacks") [12].
  - It calls the variable's own callback, then the global ones [12].
  - Installing a callback calls it once with the current value [12].
- **Flags** [10]:
  - `FCVAR_DEVELOPMENTONLY`: "Hidden in released products. Flag is removed automatically if ALLOW_DEVELOPMENT_CVARS is defined." That macro is defined only in debug builds [12].
  - `FCVAR_HIDDEN`: "Doesn't appear in find or autocomplete. Like DEVELOPMENTONLY, but can't be compiled out." Valve's wiki adds that hidden variables can still be run from config files or typed by hand (*search-recorded* [14]).
  - `FCVAR_CHEAT`: "Only useable in singleplayer / debug / multiplayer & sv_cheats".
  - `FCVAR_ARCHIVE`: saved.
  - `FCVAR_SPONLY`, `FCVAR_NOT_CONNECTED`, `FCVAR_REPLICATED`, `FCVAR_NOTIFY`, `FCVAR_USERINFO`, `FCVAR_PROTECTED`: multiplayer rules.
  - `FCVAR_RELOAD_MATERIALS`, `FCVAR_RELOAD_TEXTURES`: a change forces a reload.
  - `FCVAR_SERVER_CAN_EXECUTE`, `FCVAR_SERVER_CANNOT_QUERY`, `FCVAR_CLIENTCMD_CAN_EXECUTE`: who may run or query it.
- **Help output.** `ConVar_PrintDescription` prints, in order [12]:
  1. `"name" = "value"`;
  2. ` ( def. "..." )` when the value differs from the default;
  3. the min and max;
  4. flag words (`archive`, `cheat`, `replicated`, …);
  5. ` - help`.
  - It does not print `developmentonly`.
- **The console panel** (`consoledialog.cpp`) [13]:
  - **Matching while typing:** case-insensitive prefix. Development-only and hidden variables are skipped.
  - **The list:** sorted, at most 10 menu items, the tenth being "...". Each variable is shown with its current value beside the name.
  - **Empty line:** the list shows the command history instead.
  - **After a space:** the command's `AutoCompleteSuggest` fills the list.
  - **Keys:** Tab and Down step forward through the list. Shift+Tab and Up step back. Inserting a variable inserts only its name and a space.
  - **History:** an older duplicate is removed when a command is added again. The newest entry is at the end.
  - **Closing:** a typed `~` or `` ` `` at the end of the line closes the console unless Alt or Ctrl is held. The comment says "Alt-Tilde toggles Japanese IME on/off".
- **`find` and `help`.** In CS2, `find <text>` "will list all commands that contain the specified text in either their name or description" [17]. `help <name>` shows a variable's help without changing it (*search-recorded* [15]). Valve's wiki describes a suggestion box under the input, navigated with Up and Down and accepted with Tab (*search-recorded* [15]).
- **Hiding is not removal.** The `cvar-unhide` plugin reveals every hidden and development-only variable in CS:GO and Portal 2. It can also set values past their bounds and run commands "regardless of their hidden, cheat, or development-only flags". It needs the game started with `-insecure` [16].

---

## 3. Unreal Engine

- **Registration** [18]:
  - `TAutoConsoleVariable<T>` registers a variable at engine start. The example is `r.RefractionQuality`: an `int32`, default 2, multi-line help, flags `ECVF_Scalability | ECVF_RenderThreadSafe`.
  - `IConsoleManager::RegisterConsoleVariable` registers one inside a function.
  - `FAutoConsoleVariableRef` binds an existing C++ variable. The page advises against it: it bypasses thread safety, callbacks, sinks and cheat handling.
- **Reading** [18]. Read the variable directly with `GetValueOnGameThread()` or `GetValueOnRenderThread()`. Name lookups are slow, so cache the pointer. `ECVF_RenderThreadSafe` keeps a shadow copy that the render thread updates in order [19].
- **Change detection** [18]. Three ways, in order of preference:
  1. polling: keep the old value and compare each frame;
  2. a sink that runs on the main thread before rendering;
  3. a callback. The page discourages callbacks: they can deadlock and can fire at any time inside `Set()`, including during initialisation.
- **Who set it.** Each value records its source. A lower-priority source cannot overwrite a higher one, and the engine logs that the value "wasn't set" [18]. The full list has 16 levels [19]:
  - from Constructor, Scalability, GameSetting, ProjectSetting, SystemSettingsIni, PluginLowPriority, DeviceProfile, PluginHighPriority, GameOverride, ConsoleVariablesIni, Hotfix, Preview, Commandline, Code, Temp
  - up to Console, the highest.
  - The page calls `SetByCode` "least useful, likely a hack" [18].
- **Other flags** [19]:
  - `ECVF_Cheat` "changes the variable's behavior in final release builds".
  - `ECVF_ReadOnly`: the console cannot change it.
  - `ECVF_Scalability` and `ECVF_ScalabilityGroup`: quality groups, named `sg.`.
  - shader-change flags.
  - `ECVF_SaveForNextBoot`: saved to a temporary file after a hotfix.
- **Style** [18]:
  - "Variables should reflect user input, not system state", and code should not change them.
  - Names short but descriptive, no negation, consistent capitals.
  - Clear help text.
  - Register at start-up so that completion, `DumpConsoleCommands` and `Help` see the variable.
- **Setting a value** [18]:
  - In the console, `Name value` sets it, `Name` prints it and `Name ?` prints its help.
  - On the command line, `-ExecCmds="r.BloomQuality 12,vis 21,Quit"`, without the `=` used in ini files.
  - In `Engine/Config/ConsoleVariables.ini`, under `[Startup]`. Names are case-insensitive there and unknown names are ignored. The page says this file is for local developer use, not for project settings.
  - Engine ini files use `[SystemSettings]` sections.
- **The console's completion** [20][21]:
  - A full list of commands and variables (`AutoCompleteList`) and the current matches (`AutoComplete`).
  - A selected index, and a "locked" selection state.
  - A history buffer kept in config, so it survives between runs.
  - Project settings can show the first line of help in the completion window ("Display Help in Auto-Complete").
  - Different colours for commands, settable variables, and "command descriptions and read-only CVars".
  - The internal completion tree is built one character at a time, which suggests prefix matching. Whether the list also matches substrings was not confirmed.
- **Shipping builds.**
  - Community answers on Epic's forum describe the built-in console as stripped from Shipping builds. A `ALLOW_CONSOLE_IN_SHIPPING` define defaults to 0 [22] (*search-recorded* for the define's location in `Build.h` [23]).
  - Changing it alone was reported as not enough, because some console paths in `PlayerController.cpp` test `!UE_BUILD_SHIPPING` [23].
  - The advice in the thread is to build a separate console that checks "every single command that is entered" against an allowed list. One poster warns that otherwise players "could very easily use console commands to give themselves wall hacks" [22].
  - Community answers also report that `ECVF_Cheat` variables are disabled in packaged builds (*search-recorded* [25]). The documentation says only that the flag changes behaviour in final builds [19], and suggests compiling development features out with `#if !(UE_BUILD_SHIPPING || UE_BUILD_TEST)` [18].
- **Console key on AZERTY.** Issue UE-15894 reports that ² and other AZERTY keys could not be chosen as console keys. It was closed as Won't Fix (*search-recorded* [24]).

---

## 4. CryEngine (and Lumberyard's ancestor)

- **Flags** (`EVarFlags`, CryEngine 3's `IConsole.h`) [26]:
  - `VF_CHEAT`: "stays in the default state when cheats are disabled".
  - `VF_DEV_ONLY`: "cvar is only registered with the console in non release builds".
  - `VF_DEDI_ONLY`: registered only in non-release or dedicated-server builds.
  - `VF_READONLY`: "can not be changed by the user".
  - `VF_REQUIRE_LEVEL_RELOAD` and `VF_REQUIRE_APP_RESTART`: no comment in the header.
  - `VF_RESTRICTEDMODE`: "is visible and usable in restricted (normal user) console mode".
  - `VF_INVISIBLE`: "Invisible to the user in console".
  - `VF_CONST_CVAR`: "a const cvar not to be set inside cfg-files".
  - `VF_ALWAYSONCHANGE`: call the change callback even when the value did not change.
  - `VF_WASINCONFIG` and `VF_WARNING_NOTUSED`: track whether a config file set the variable.
  - `VF_CHEAT_ALWAYS_CHECK` and `VF_CHEAT_NOCHECK`: an anti-tamper hash of cheat variables.
  - `VF_DEPRECATED`: "use default values which cannot be modified outside the code".
- **Restricted mode.** `con_restricted` is itself a console variable flagged `RESTRICTEDMODE`. Its help reads "0=normal mode / 1=restricted access to the console" [28].
- **Sinks.** `IConsoleVarSink::OnBeforeVarChange` returns false to refuse a change. `OnAfterVarChange` follows the change (*search-recorded* [29]). A command or variable can register an argument completer through `RegisterAutoComplete` (*search-recorded* [29]).
- **Use** (CryEngine 3) [27]:
  - The console opens on `~` (`^` on German keyboards).
  - Tab completes, and repeated Tabs cycle through the matches. Up and Down browse the history.
  - `name ?` prints help, and `?word` lists matching variables.
  - `.cfg` files hold `name value` lines (with an optional `=`, comments `--`) and run with `exec`. They load in order: `%USER%/game.cfg`, `root/system.cfg`, `root/user.cfg`.
  - CVar groups are `.cfg` files that set many variables from one numbered state. Their main use is the `sys_spec` quality level (*search-recorded* for the CryEngine 5 wording [30]).
- **`r_DisplayInfo`.** CryEngine's usage line is `r_DisplayInfo [0=off/1=show/2=enhanced]`. In CryEngine 5.6, value 1 shows frame rate, visible triangles, lights and draw calls (*search-recorded* [31][32]). Star Citizen's version is in §8.

---

## 5. O3DE

- **Flags** (`ConsoleFunctorFlags`, `IConsoleTypes.h`) [33]:
  - `ReadOnly`: "Should not be invoked at runtime".
  - `IsCheat`: "Command is a cheat, may require escalated privileges to modify".
  - `IsInvisible`: "Should not be shown in the console for autocomplete".
  - `IsDeprecated`: "show a warning when invoked".
  - `NeedsReload`: "Level should be reloaded after executing this command".
  - `DontReplicate`, `ServerOnly`, `AllowClientSet`, `DontDuplicate`: networking and duplicates.
  - The user guide marks `DontReplicate`, `ServerOnly` and `AllowClientSet` as "CURRENTLY UNUSED" [34]. It also describes `IsCheat` as hiding a command from completion, which disagrees with the header. The header's comments are the more precise source.
- **Declaration** [34]:
  - `AZ_CVAR(type, name, default, callback, flags, description)`, in `.cpp` files only. Supported types are bool, integers, floats, vectors, quaternions and enums. Custom types need two conversion functions.
  - "These macros do not guarantee that this callback will be run on a specific thread."
  - Functions are registered with `AZ_CONSOLEFUNC` and `AZ_CONSOLEFREEFUNC`.
  - Suggested prefixes are `sv_`, `cl_` and `bg_` (server, client, both), "to limit autocomplete scope".
  - The page promises "Basic access protections and anti-cheat mechanisms for locking down cvars and cfuncs in release builds" without describing them.
- **Start-up order** [35]:
  1. `.setreg` files load before any module.
  2. `user.cfg` runs after modules load but before they activate.
  3. Command-line commands (`-<command> <args>`) run last.
  - Commands run in file order. "Console commands that are deferred might not run in the same order": a command for a variable not yet registered waits.
  - Variables can also be set with `--name value` or `--name=value` (*search-recorded* [36]).

---

## 6. Rust and Bevy

- **`cvars`** (martin-t) [41]:
  - The `cvars!` macro or `#[derive(SetGet)]` turns a struct's fields into variables.
  - Game code reads plain fields. The console uses generated `set_str` and `get_string` by name.
  - It supports user-defined struct and enum types, saving and loading files, completion, and consoles for Fyrox and Macroquad.
  - It has no flags, ranges or callbacks.
  - Its README argues against hash maps: other tweak crates "use hashmaps", which add "overhead on every access".
- **`cvar`** (CasualX) [42]:
  - A visitor trait (`IVisit`) exposes `Property`, `ClampedProp` (clamped to inclusive bounds), `ReadOnlyProp`, `OwnedProp`, `List` and `Action` nodes.
  - Values are parsed and printed with `FromStr` and `Display`.
  - `repl::set` and `repl::invoke` act by name.
- **`bevy_console`** [37][38][39]:
  - A "Half-Life inspired console": commands are clap structs. The doc comments become the help.
  - It has history, completion, ANSI colours and log capture, drawn with `bevy_egui`.
  - **Toggle:** by physical `KeyCode`, `Backquote` by default. The example adds the Swedish key under Esc by native scancode 41 on Windows, macOS, X11 and Android, plus F1 [39].
  - **Completion:** a trie gives prefix predictions (four shown). Tab cycles through them, and Enter on a highlighted suggestion fills the line without running it [38].
  - **History:** Up and Down over 20 entries [38].
  - **Keys:** Ctrl+C clears the line, Ctrl+L clears the scrollback [37].
  - The source has no explicit filter that stops the backquote from reaching the text field. It relies on egui focus [38].
- **`bevy_dev_console`** [40]. A "Source-inspired" console with its own small language (variables, calculations, resource editing). It opens on `` ` ``/`~`. It was in early development on Bevy 0.14, available only from git.
- **`bevy-inspector-egui`** [43]. An egui inspector over Bevy's reflection, with field ranges such as `#[inspector(min = 0.0, max = 1.0)]`. It is a property panel, not a console.
- **Static registration from many crates:**
  - `inventory` gathers values submitted from any linked crate, in no particular order. Its page states support only for Linux x86-64 and WebAssembly [44].
  - `linkme`'s `distributed_slice` has the linker gather `static` items into one `&'static [T]`. Its table lists Linux, macOS and Windows [45]. Elements must be `const`.
  - This is the Rust form of Doom 3's statically declared `idCVar`s [8].

---

## 7. Completion and history: what feels best

**What the references do:**

| | Matching | Tab | Candidate list | Values and help in the list | Arguments |
|---|---|---|---|---|---|
| Quake III [3] | prefix, case-insensitive | common prefix | printed on Tab | names only | no |
| Doom 3 [9] | prefix | common prefix, then cycle | printed on first Tab | variable values | yes (strings, booleans, files) |
| Source [13] | prefix, case-insensitive | cycle (Shift+Tab back) | live, ≤ 10 items | values | yes (command callbacks) |
| Unreal [20][21] | prefix tree (substring not confirmed) | select from list | live | first line of help, colours by kind | via delegates |
| CryEngine [27] | prefix | cycle | — | — | via registered completers [29] |
| bevy_console [38] | trie prefix | cycle | live, 4 items | no | fixed sequences |
| GNU Readline [50] | prefix (case-insensitive optional) | `complete`: common prefix; `menu-complete`: cycle | on second Tab or at once (`show-all-if-ambiguous`) | — | per program |
| fish [51] | — | one match inserted; else common prefix + pager | pager, searchable with Ctrl+S | descriptions | per command |

**Common prefix or cycling?**
- Both are needed.
- The common prefix is the right first step, as in Quake III, Doom 3, Readline and fish [3][9][50][51]. It never guesses. With dotted names it stops at a meaningful boundary: `gi.pr` + Tab gives `gi.probe` when `gi.probe_spacing` and `gi.probe_rays` exist.
- Once the prefix cannot grow, the next Tab should cycle through the candidates, as Doom 3 and Readline's `menu-complete` do [9][50]. Shift+Tab should cycle backwards, as in Source [13].
- A unique match should add a trailing space, as in Quake III and Source [3][13], so the next Tab completes the value.

**A live list, with values and help.**
- Source and Unreal show the candidates while you type [13][20]. Doom 3 prints values [9]. Unreal adds the first line of help [20].
- This is what makes a console discoverable. The owner does not need to remember `gi.probe_spacing`'s range or what `r.display_info 2` shows.
- Source's cap of about ten items with a "..." entry keeps the list readable [13].

**Completing the value.**
- Doom 3 completes allowed strings and booleans after the name [8][9]. Source asks the command for suggestions after the first space [11][13].
- Choice and boolean variables should complete their values the same way, with the current one marked. Numbers cannot be listed, so the line under the input should show the range, the default and the current value instead.

**Prefix, substring or fuzzy?**
- Every engine above matches by prefix [3][9][13][27][38].
- For a search, Source and CS2 provide `find`, which matches text anywhere in names and descriptions [17]. CryEngine has `?word` [27].
- A fuzzy subsequence match (letters in order, gaps allowed) was not found in any game console reviewed. With a few hundred dotted names it would flood the list and make the common prefix meaningless.
- *Judgment:* match by prefix first. When nothing matches, match the typed text at the start of any dotted segment (`probe` finds `gi.probe_spacing`), then anywhere. Keep `find` for searches in help text.

**History.**
- Up and Down recall earlier lines in all the engines and shells reviewed [13][27][38][50][51].
- fish filters history by what is already typed: "only the commands containing the string entered into the command line are shown" [51]. Readline offers the same by prefix as `history-search-backward` [50].
- Source removes an older duplicate when a line is added again [13].
- Unreal keeps the history in config across runs [21]. History only recalls lines and replays nothing, so keeping it does not contradict the owner's "console changes are temporary". It is still listed as a point for the owner in §10.8.
- Source reuses Up and Down for the candidate list when the line is not empty [13]. *Judgment:* that makes Up ambiguous. Tab and Shift+Tab should move through candidates, and Up and Down should only ever move through history.

---

## 8. Release builds and anti-cheat

- **Star Citizen** [46]:
  - The console opens on `~` at any time.
  - `r_DisplayInfo` has five levels:
    - 0: off;
    - 1: "Shows some basic information";
    - 2: "additional info (Session, Shard ID, CPU load, etc) and everything above";
    - 3: "(RAM, VRAM usage, etc.) and everything above";
    - 4: "GPU Load stats and everything above".
  - `r_DisplaySessionInfo 1` draws a QR code that CIG can use in bug reports. The game's settings also expose this session code [47].
  - The community wiki says that `Con_restricted = 0` in `USER.cfg` "allows for more commands" [46]. That is CryEngine's `con_restricted` [28]. If true for the live build, the restriction is a variable that a user's file can switch off.
- **Source and CS2.**
  - `FCVAR_CHEAT` variables work only in single-player, debug builds, or with `sv_cheats` on the server [10].
  - Development-only variables are hidden from completion and `find` in release builds [10][13]. A plugin can still reveal them if the game starts in `-insecure` mode [16].
  - The gate is a run-time switch plus hiding. Neither is removal.
- **Unreal.** The built-in console is not in Shipping builds by default [22][23]. Projects that want one in Shipping build their own, with an allowed list of commands [22].
- **CryEngine.** `VF_DEV_ONLY` variables are not registered at all outside development builds [26]. This is the only engine reviewed whose release build removes development variables entirely.
- **Minecraft** [48]:
  - Its F3 screen is information only: frame rate, coordinates, chunk statistics, memory, biome, the targeted block.
  - The `reduced_debug_info` game rule hides some of those lines, including the player's coordinates. A 2014 change made the rule override "all 'reduced debug info' settings in clients".
  - Lesson: information can itself be an advantage in multiplayer, and the server may need to restrict it.
- **Valheim.** The console needs the `-console` launch option, or since 2025 a setting. Cheat commands need `devcommands`. Guides disagree on whether those work on dedicated servers (*search-recorded* [49]).
- **What follows for Forge:**
  - A shipping build that contains a development variable can be made to use it: by a plugin [16], a config file [46] or a startup-order bug [7].
  - Removing it at compile time, as CryEngine does with `VF_DEV_ONLY` [26] and Unreal does with the whole console [22][23], leaves nothing to unlock.
  - "Information-only" must mean information about the machine, the build and the session: frame rate, frame and pass times, CPU, GPU and memory load, build version, session code. It must not mean information about the world, such as positions, hidden objects, other players or entity lists [48].
  - A client-side restriction does not stop memory editing. Real anti-cheat in multiplayer needs server authority, which this note does not cover.

---

## 9. winit 0.30 text input

**Facts:**

- **The key under Esc.**
  - `KeyEvent.physical_key` "Represents the position of a key independent of the currently active layout" [52].
  - `KeyCode::Backquote` is documented as "` on a US keyboard" and the hankaku/zenkaku/kanji key on Japanese keyboards [53]. This follows the W3C rule that a code is named after the US key at that position [59].
  - winit's Windows backend maps scancode `0x0029` to `KeyCode::Backquote` and `0x0056` to `IntlBackslash` [57]. MDN gives the same mapping for browsers [60].
  - SDL documents that scancode as the key "in the top left corner (on both ANSI and ISO keyboards)". It produces grave and tilde on a US Windows layout, and "SUPERSCRIPT TWO" on a French Windows layout. It produces circumflex and degree on a German layout [61].
  - The French Windows layout puts virtual key `OEM_7` on scancode 29 [62]. Shift+² types nothing [63].
  - Two cases are different: Swiss and German Mac layouts on ANSI keyboards produce `<`/`>` there, and Mac ISO keyboards give `§` [61]. They do not matter for Forge on Windows.
  - So `PhysicalKey::Code(KeyCode::Backquote)` is the key under Esc on the owner's AZERTY and on US keyboards.
- **Text.**
  - `KeyEvent.text` holds "the text produced by the keypress" and is `None` when the press makes no text. Enter gives `"\r"` [52].
  - `logical_key` is "affected by all modifiers except Ctrl", and so is `text` [52]. Ctrl+L still carries `"l"`.
  - `Event::ReceivedCharacter` was removed in 0.29. Text is now `KeyEvent.text` together with `WindowEvent::Ime` [58]. The toggle key's character therefore arrives in the toggle key's own event, and swallowing that event keeps it out of the line.
- **Dead keys.**
  - A dead key's press has `logical_key` `Key::Dead(Option<char>)` and no text [54][57].
  - The next key gives the composed text. On Windows, when the two cannot combine, `text` holds both characters: the accent, then the letter [52].
  - On AZERTY, `^` (right of P) and the AltGr accents are dead keys. Typing `^` then a space gives `^`, and `^` then `e` gives `ê`. winit's `text` already handles this. The console only inserts `text`.
- **AltGr.** On Windows, AltGr arrives as a fake Ctrl followed by right Alt. winit drops the fake Ctrl event, and when Alt is also down Ctrl does not count as a modifier for text [57]. AZERTY users type `~ # { [ | \ ^ @ ] }` with AltGr. A shortcut rule of "Ctrl without Alt" leaves them typable.
- **IME** [55][56]:
  - IME is "**not** allowed by default". Without it, the window "will receive KeyboardInput events for every keypress instead".
  - Once allowed with `Window::set_ime_allowed(true)`, the window receives `Ime::Enabled`, `Preedit(text, cursor)`, `Commit(text)` and `Disabled`. "During the preedit phase the window will NOT get KeyboardInput events".
  - `set_ime_cursor_area` places the IME candidate window.
  - On macOS, "IME must be enabled to receive text-input where dead-key sequences are combined". On X11, enabling IME disables dead-key reporting during composition [56].
- **Key repeat.** `KeyEvent.repeat` is true for the operating system's auto-repeat [52].

**What Forge's handler should do** (a sketch for `forge-app/src/lib.rs`, after the calibration pages):

```rust
WindowEvent::KeyboardInput { event, .. } => {
    let PhysicalKey::Code(code) = event.physical_key else { return };
    let pressed = event.state == ElementState::Pressed;
    // 1. The toggle: the key under Esc, by position. Its text (², `) never reaches the line.
    if code == KeyCode::Backquote {
        if pressed && !event.repeat {
            state.console.toggle(&state.ctx.window); // set_ime_allowed(open)
            // A dead toggle key (German ^) leaves the OS waiting for the next key.
            state.console.strip_next = match &event.logical_key {
                Key::Dead(c) => *c,
                _ => None,
            };
        }
        return;
    }
    // 2. While open, the console takes every key; releases still reach `Input`, so a key
    //    held when the console opened does not stay down for the camera.
    if state.console.is_open() {
        if !pressed {
            state.input.set_key(code, false);
            return;
        }
        state.console.key(&event, state.modifiers); // see below
        return;
    }
    // ... today's handling (Escape quits, F1, Alt+Enter, ...)
}
WindowEvent::ModifiersChanged(m) => state.modifiers = m.state(),
WindowEvent::Ime(Ime::Commit(text)) if state.console.is_open() => state.console.insert(&text),
WindowEvent::Ime(Ime::Preedit(text, cursor)) if state.console.is_open() => {
    state.console.preedit(text, cursor)
}
```

Inside `Console::key`:
1. **Named keys first** (`Key::Named` from `logical_key`):
   - Enter runs the line. Escape closes the console and never quits the app while it is open.
   - Tab and Shift+Tab complete. Up and Down move through history.
   - Left, Right, Home and End move the cursor. Backspace and Delete edit. Page Up and Page Down scroll.
   - Repeats are accepted for editing and movement keys.
2. **Shortcuts.** When Ctrl is held without Alt: Ctrl+L clears the scrollback, Ctrl+U clears the line, Ctrl+Left and Ctrl+Right jump words. No text is inserted.
3. **Text.** Otherwise insert `event.text`:
   - Drop control characters.
   - If `strip_next` is `Some(c)` and the text has two or more characters starting with `c`, drop that first character. This is the dead-toggle case on Windows [52]. A composed letter such as `ê` cannot be split back and is kept.
   - Keep only the overlay's printable ASCII (codes 32 to 126). Names and values are ASCII, and the font atlas has no other glyphs. Ignore any other character, with a one-line hint the first time.
4. On opening, clear the held movement keys in `Input`. On closing, call `set_ime_allowed(false)` so that game keys never go to an IME.

---

## 10. For Forge

### 10.1 The variable: types, flags, storage

Variables are `static` items, declared where they are used, as in Doom 3 [8] and Source [11]. `linkme` gathers them into one slice at link time [45]. The full set is therefore known before `main` reads the command line. The benefits:
- an unknown name is an error at once;
- completion is complete from the start;
- `--list-vars` works without starting Vulkan;
- no value has to wait for a late registration, which O3DE has to handle [35].

Values are atomics, so any thread reads one with a relaxed load. That is a field access in cost, which answers the `cvars` crate's objection to hash maps [41].

```rust
// crates/forge-app/src/vars.rs (sketch)
pub enum Kind {
    Bool,                                // on/off, true/false, 1/0
    Int { min: i64, max: i64 },
    Float { min: f64, max: f64 },
    Choice(&'static [&'static str]),     // set by name or by index: r.display_info 2
    Text,                                // stored apart, behind a Mutex<String>
}

#[derive(Clone, Copy)]
pub struct Flags(u8);
impl Flags {
    /// No flag: a development variable. In shipping builds it has no name and reads its default.
    pub const DEV: Flags = Flags(0);
    /// Kept in shipping builds. Information about the machine, the build or the session only.
    pub const RELEASE: Flags = Flags(1 << 0);
    /// Shows engine state (the version, the GPU); only the engine sets it.
    pub const READ_ONLY: Flags = Flags(1 << 1);
    /// Read once at start-up: `forge.cfg` and `--set` only. The console says how to set it.
    pub const RESTART: Flags = Flags(1 << 2);
    /// Not listed or completed: old flag spellings kept as aliases during the move.
    pub const HIDDEN: Flags = Flags(1 << 3);
}

pub struct Var {
    pub name: &'static str,
    pub help: &'static str,     // the declaration's doc comment
    pub kind: Kind,
    pub flags: Flags,
    default: u64,               // bool 0/1, i64, f64::to_bits, choice index
    value: AtomicU64,
    serial: AtomicU32,          // +1 per applied change (Quake III's modificationCount)
    set_by: AtomicU8,           // Default, Config, CommandLine, Console
}

#[linkme::distributed_slice]
pub static VARS: [&'static Var];
```

Typed wrappers (`BoolVar`, `IntVar`, `FloatVar`, `ChoiceVar<E>`, `TextVar`) are declared by one macro. The doc comment becomes the help line, as in `bevy_console` [37]:

```rust
forge_app::var! {
    /// Anti-aliasing: off, taa, dlaa (the default), ssaa.
    pub static R_AA: Choice<Aa> = ("r.aa", Aa::Dlaa);
    /// Frame information: 0 off, 1 frame rate and time, 2 passes, CPU, GPU and memory.
    pub static R_DISPLAY_INFO: Choice<DisplayInfo> = ("r.display_info", DisplayInfo::Off, RELEASE);
    /// Distance between light probes, in metres.
    pub static GI_PROBE_SPACING: Float = ("gi.probe_spacing", 2.0, 0.25..=8.0);
    /// The monitor the window opens on: primary or secondary.
    pub static APP_MONITOR: Choice<Monitor> = ("app.monitor", Monitor::Secondary, RESTART);
}
```

- **Reading.** `R_AA.get()` returns `Aa`. In a shipping build, a variable without `RELEASE` returns its constant default, so the compiler can fold the read:

  ```rust
  if cfg!(feature = "shipping") && !RELEASE { return DEFAULT }
  ```

- **Change detection: polling, no callbacks.** This follows Quake III's counter [2] and Unreal's advice [18]:

  ```rust
  if GI_PROBE_SPACING.changed(&mut self.probe_serial) { self.rebuild_probes(); }
  ```

- **When changes apply.** A console change is queued and applied once per frame, at the top of the frame, before any system reads. Values are therefore stable for a whole frame, on every thread. This is the role of Unreal's sinks and render-thread shadow copies [18][19].
- **Values.** A value outside its range, or not among its choices, is refused with a message that shows the range or the choices. It is not clamped as in Source [12], because a silent clamp hides typos (`10` for `1.0`).
- **Names.** Lowercase ASCII letters, digits and `_`, in dot-separated segments. Lookup is case-insensitive. A test checks that names are unique, well formed and have a help line.

### 10.2 Naming groups

| Group | Holds | Examples (illustrative) |
|---|---|---|
| `app.` | window, monitor, timing, frames, captures, version | `app.monitor` (RESTART), `app.fixed_step`, `app.frames`, `app.version` (READ_ONLY, RELEASE) |
| `r.` | rendering | `r.aa`, `r.shadows`, `r.ao`, `r.reflections`, `r.clouds`, `r.exposure.local`, `r.hdr`, `r.debug_view`, `r.display_info` (RELEASE) |
| `gi.` | probes and global illumination | `gi.probes`, `gi.probe_spacing`, `gi.probe_rays`, `gi.regions` |
| `ui.` | the overlay and the console | `ui.overlay.font_px`, `ui.console.lines` |
| `phys.` | physics | `phys.substeps`, `phys.debug_draw` |
| `net.`, `audio.`, `sim.` | later systems | — |
| one per demo | the demo's own scene | `city.`, `planet.`, `island.`, `lab.`, `asteroids.` |

Rules:
- Positive names with a value, never negations. Unreal's guide says to avoid negation [18]: `r.aa off`, not `--no-taa`.
- Units go in the help line. They go in the name only where it would otherwise be ambiguous (`_ms`, `_m`).
- One group per system, so that a prefix and Tab list one system's settings. O3DE gives the same reason for its prefixes [34].

### 10.3 The command line and `forge.cfg`

- **Command line** (added to each demo's clap parser):
  - `--set NAME=VALUE`, repeatable; the last one wins.
  - `--cfg PATH`: read another file.
  - `--no-cfg`: read no file. This plays the role of Quake III's safe mode [3].
  - `--list-vars`: print every variable with its default, range, flags and help, then exit.
  - Today's flags become aliases of `--set` during the move (`--dlaa` is `--set r.aa=dlaa`), as D-069 says.
  - An unknown name or a bad value on the command line is an error. The program exits and names the nearest variables, so scripts fail loudly.
- **`forge.cfg`:**
  - A hand-edited file in the working directory, ignored by git. Forge never writes it, as the owner decided.
  - One `name = value` per line, with `#` comments.
  - An optional `[demo]` section applies only to that demo:

    ```
    # forge.cfg: read at start-up, never written by Forge.
    r.aa = dlaa
    gi.probe_spacing = 1.0
    [city-plan]
    city.crowd = 2000
    ```

  - An unknown name in the file is a warning, not an error. Demos link different crates, so one file can name variables that a given demo does not have. Unreal ignores unknown names in `ConsoleVariables.ini` [18]; Forge warns.
- **Order.** Each later source overrides the earlier ones:
  1. defaults;
  2. `forge.cfg` (its global part, then the demo's section);
  3. the command line, in order;
  4. the console.

  Each variable records the last source that set it, a four-level form of Unreal's set-by record [18][19]. `help` shows it.
- **Scripts.**
  - `tools/captures.sh`, `tools/map-check.sh`, `tools/verify.sh` and the timings must pass `--no-cfg`. Otherwise the owner's personal file changes accepted images.
  - Each capture manifest should record the variables that are not at their defaults (the `changed` command's output). Each image then states its settings.

### 10.4 The console's commands

| Command | Does | In shipping |
|---|---|---|
| `NAME` | prints value, default, choices or range, source, help line | RELEASE variables |
| `NAME VALUE` (or `set NAME VALUE`) | sets it; prints `r.aa = taa (was dlaa; default dlaa)` | RELEASE variables |
| `NAME ?`, `help NAME` | full help, as Source's description [12] and Unreal's `?` [18] | yes |
| `help` | how to use the console, then the commands | yes |
| `find WORD` | variables and commands with WORD in the name or help, as in CS2 [17] | yes |
| `list [PREFIX]` | names, values and flag letters, as Quake III's `cvarlist` [2] | yes |
| `changed` | every variable not at its default, with its source | — |
| `reset NAME` / `reset PREFIX.` / `reset all` | back to the default or to the start-up value | — |
| `toggle NAME` | flips a boolean or steps a choice, as Quake III's `toggle` [2] | — |
| `exec FILE` | applies a `.cfg` file for this run only | — |
| `version` | build version, commit, date, dev or shipping | yes |
| `clear` | clears the scrollback | yes |
| demo and system commands | `capture`, `view NAME`, debug views (D-069) | — |

- **Commands are registered at run time.** They need the app's or demo's state, so they are not statics. Each has a name, a usage line, a help line, flags (the same `RELEASE`) and an optional argument completer. Running one queues an action that the frame loop carries out.
- **Refusals say why:**
  - "outside 0.25..8 (unchanged: 2)";
  - "read-only";
  - "`app.monitor` is read at start-up: use `--set app.monitor=primary` or `forge.cfg`";
  - "unknown variable; did you mean `gi.probe_spacing`?".
- **`RESTART` means start-up only.** Quake III can latch a value for the next start [1][2], but only because it saves values. Forge saves nothing, so a latched value would be lost at exit.

### 10.5 Completion and history in Forge's console

1. **While typing** (one character or more), show up to 10 candidates under the line, then "… and N more" [13]. Each line reads `name  value  first line of help`, in the overlay's colours: settable, read-only, command [20].
2. **Tab:**
   - one candidate: complete it and add a space;
   - several: extend to their longest common prefix [3][9];
   - if the prefix cannot grow: cycle forward, Shift+Tab back [9][13].
3. **Matching:** case-insensitive prefix. If nothing matches, the start of any dotted segment, then anywhere. No fuzzy subsequence.
4. **After `NAME `:** choices or on/off, with the current value marked [8][9]. For numbers, a hint line shows `range 0.25..8, default 2, now 1`. After a command name, the command's own completer runs (`view`: view names; `exec`: `.cfg` files).
5. **Up and Down: history only.**
   - Newest first, up to 100 entries, older duplicates removed [13].
   - If the line had text when Up was first pressed, only entries containing it are shown [51].
6. **Drawing.** The F1 overlay's text cells over the top third of the window, with a 500-line scrollback (D-069). Nothing is drawn or computed while the console is closed, and the F1 overlay should show that its cost is zero.

### 10.6 `r.display_info` and the F1 overlay

- The F1 overlay already has three modes: Off, Compact and Full (`profile.rs`). `r.display_info` should drive them: 0 off, 1 compact, 2 full, settable by name or number like Star Citizen's levels [46]. F1 then steps `r.display_info`.
- In shipping builds the overlay keeps machine, build and session lines only (§8). Any world line, such as camera coordinates, stays behind a development variable [48].
- `app.version` is `READ_ONLY | RELEASE`. Embedding the commit needs a build script. It should live in a leaf crate and be checked against the world products' cache keys, because build-script edits have remade every product before.

### 10.7 How release builds strip development variables

- **The switch.** A cargo feature `shipping` on `forge-app`, off by default and forwarded by the demos. It is not the cargo `release` profile: timings and captures use optimised builds that need every development variable.
- **With `shipping` on:**
  1. The registry keeps only `RELEASE` variables and commands. Other names do not exist for `--set`, `forge.cfg`, the console, `list`, `find` or completion. The reply is "unknown variable", not "cheat protected", so the build does not reveal which names exist.
  2. `get()` on any other variable returns its constant default. Under D-069 the defaults are the best look, so a shipping build looks like the default development build.
  3. Development-only code that would help a player (free camera, time scale, captures, debug views, teleports) is compiled out with `#[cfg(not(feature = "shipping"))]`, as Unreal's guide suggests for its builds [18].
  4. A CI job builds with `--features shipping`, runs `--list-vars` and compares the result with `tools/release-vars.txt`. Adding `RELEASE` to a variable is then always a reviewed change.
- **The default is private.** A variable without a flag is a development variable, as in Doom 3, where every variable is a cheat unless flagged [8]. A forgotten flag fails safe.

### 10.8 Open points for the owner

- Keep the console history in a file between runs, which recalls lines and replays nothing? Unreal does [21].
- Where `forge.cfg` lives: the working directory, as proposed, or next to the executable.
- Whether a shipping build also reads `forge.cfg` (for its `RELEASE` variables only) or ignores it.

### 10.9 Tests

- **The registry:**
  - parsing of each kind (bool spellings, choices by name and index);
  - ranges;
  - unknown names with suggestions;
  - the order defaults, then `forge.cfg`, then command line, then console;
  - aliases;
  - `RESTART` and `READ_ONLY` refusals.
- **Completion:** common prefix, cycling, Shift+Tab, value completion, segment matching.
- **Shipping:** a test under `--features shipping` checks that no development name resolves and that `get()` returns defaults.
- **Captures:** each demo's captures before and after the move, as D-069 plans.

---

## 11. What Forge should not copy

- **Creating a variable from an unknown name.** Quake III's `set` makes a user variable from a typo [1][2]. Forge refuses and suggests the nearest names.
- **Saving changes automatically.** Quake III's `seta` and the rewrite of `q3config.cfg` [2][3], and archive flags generally [10][26], are ruled out by the owner.
- **A run-time cheat switch.** `sv_cheats` resets, latched cheat variables escaping the reset, and startup-order bugs [2][7] are all avoided by compiling the gate.
- **Hiding instead of removing.** Source's `FCVAR_HIDDEN` and `FCVAR_DEVELOPMENTONLY` are revealed by a plugin [16]. CryEngine's `con_restricted` is a variable, and a user file reportedly switches it off [28][46].
- **String storage parsed on every set** [1][2]. Forge stores typed values.
- **Silent clamping** [12]. Forge refuses with the range.
- **Change callbacks.** They can deadlock or run on any thread [18][34]. Forge polls a counter.
- **Unreal's 16 set-by priorities** [19]. Four sources in a fixed order are enough. The source is recorded for `help`.
- **Binding a raw variable** (`FAutoConsoleVariableRef`), which bypasses thread safety [18].
- **Matching the console key by character or key name.** This caused ioquake3's console-key problems [4][5][6] and Unreal's ² issue [24].
- **Completing only the first word** [3]. Forge completes values too.
- **Fixed completion buffers** (64 × 64) [11]. Not needed in Rust.
- **Replication flags, competitive bounds and cheat hashing** [10][12][26][33]. Not now. Networked settings belong to `forge-net`'s server authority.
- **`+` on the command line** [3]. It does not fit clap. Forge uses `--set name=value`.

---

## Sources

1. id Software. Quake III Arena source, `code/game/q_shared.h` (GPL-2.0). *read* — https://raw.githubusercontent.com/id-Software/Quake-III-Arena/master/code/game/q_shared.h
2. id Software. Quake III Arena source, `code/qcommon/cvar.c` (GPL-2.0). *read* — https://raw.githubusercontent.com/id-Software/Quake-III-Arena/master/code/qcommon/cvar.c
3. id Software. Quake III Arena source, `code/qcommon/common.c` (GPL-2.0). *read* — https://raw.githubusercontent.com/id-Software/Quake-III-Arena/master/code/qcommon/common.c
4. ioquake3 SVN r1470 (`cl_consoleKeys`), quake3-commits list, September 2008. *read* — https://icculus.org/pipermail/quake3-commits/2008-September/001392.html
5. Tony J. White. "internationally accepted toggleconsole bind", quake3 mailing list, February 2007. *read* — https://icculus.org/pipermail/quake3/2007-February/001599.html
6. ioquake3 Bug 4784, "backquote (`) doesn't behave like in idq3", October 2010. *read* — https://icculus.org/pipermail/quake3-bugzilla/2010-October/001641.html
7. ioquake3 Bug 4974, "overzealous cvar cheat protection", 2011. *read* — https://bugzilla.icculus.org/show_bug.cgi?id=4974
8. id Software. DOOM 3 source, `neo/framework/CVarSystem.h` (GPL-3.0). *read* — https://raw.githubusercontent.com/id-Software/DOOM-3/master/neo/framework/CVarSystem.h
9. id Software. DOOM 3 source, `neo/framework/EditField.cpp` (GPL-3.0). *read* — https://raw.githubusercontent.com/id-Software/DOOM-3/master/neo/framework/EditField.cpp
10. Valve. Source SDK 2013, `src/public/tier1/iconvar.h`. *read* — https://raw.githubusercontent.com/ValveSoftware/source-sdk-2013/master/src/public/tier1/iconvar.h
11. Valve. Source SDK 2013, `src/public/tier1/convar.h`. *read* — https://raw.githubusercontent.com/ValveSoftware/source-sdk-2013/master/src/public/tier1/convar.h
12. Valve. Source SDK 2013, `src/tier1/convar.cpp`. *read* — https://raw.githubusercontent.com/ValveSoftware/source-sdk-2013/master/src/tier1/convar.cpp
13. Valve. Source SDK 2013, `src/vgui2/vgui_controls/consoledialog.cpp`. *read* — https://raw.githubusercontent.com/ValveSoftware/source-sdk-2013/master/src/vgui2/vgui_controls/consoledialog.cpp
14. Valve Developer Community. "Console command flags" and "Developer Console Control". *search-recorded* — https://developer.valvesoftware.com/wiki/Console_command_flags ; https://developer.valvesoftware.com/wiki/Developer_Console_Control
15. Valve Developer Community. "Developer console". *search-recorded* — https://developer.valvesoftware.com/wiki/Developer_console
16. saul. `cvar-unhide` README (Source 1 plugin). *read* — https://github.com/saul/cvar-unhide
17. Total CS. "CS2 & CS:GO find Command". *read* — https://totalcsgo.com/commands/find
18. Epic Games. "Console Variables C++ in Unreal Engine". *read* — https://dev.epicgames.com/documentation/en-us/unreal-engine/console-variables-cplusplus-in-unreal-engine
19. Epic Games. `EConsoleVariableFlags`, Unreal Engine API reference. *read* — https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/Core/EConsoleVariableFlags
20. Epic Games. "Console Settings in the Unreal Engine Project Settings". *read* — https://dev.epicgames.com/documentation/unreal-engine/console-settings-in-the-unreal-engine-project-settings
21. Epic Games. `UConsole`, Unreal Engine API reference. *read* — https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Engine/UConsole
22. Epic Developer Community forum. "Enable console in shipping build" (community answers; no Epic staff in the thread). *read* — https://forums.unrealengine.com/t/enable-console-in-shipping-build/353468
23. Unreal Engine Answers. "ALLOW_CONSOLE define usage" and "how can I enable the game console in a shipping build?" (`ALLOW_CONSOLE_IN_SHIPPING` in `Build.h`, `!UE_BUILD_SHIPPING` in `PlayerController.cpp`). *search-recorded* — https://answers.unrealengine.com/questions/542664/view.html ; https://answers.unrealengine.com/questions/401991/view.html
24. Epic Games. Issue UE-15894 (AZERTY keys such as ² not available as console keys; Won't Fix). *search-recorded* — https://issues.unrealengine.com/issue/UE-15894
25. Unreal Engine Answers. Threads on `ECVF_Cheat` in packaged builds and ini files. *search-recorded* — https://answers.unrealengine.com/questions/587194/view.html ; https://answers.unrealengine.com/questions/761652/view.html
26. Crytek. CRYENGINE 3 documentation, "Console_Enums" (`EVarFlags` from `IConsole.h`). *read* — https://www.cryengine.com/docs/static/engines/cryengine-3/categories/17399809/pages/17074566
27. Crytek. CRYENGINE 3 documentation, "Using Console and Config Files". *read* — https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1638401/pages/1605736
28. Crytek. CRYENGINE 3 documentation, `CON_` console variables (`con_restricted`). *read* — https://www.cryengine.com/docs/static/engines/cryengine-3/categories/9895942/pages/9215949
29. Crytek. CRYENGINE 3 and 5 API documentation, `IConsoleVarSink` and `IConsole::RegisterAutoComplete`. *search-recorded* — https://cryengine.com/docs/static/engines/cryengine-3/categories/17399809/pages/17074729 ; https://www.cryengine.com/docs/static/engines/cryengine-5/categories/28704770/pages/29797233
30. Crytek. CRYENGINE 5 documentation, "Console Variables & Config Files" (CVar groups, `sys_spec`). *search-recorded* — https://www.cryengine.com/docs/static/engines/cryengine-5/categories/23756816/pages/25535264
31. Crytek. CRYENGINE documentation, "Game Executable Launcher" (`r_DisplayInfo` usage). *search-recorded* — https://www.cryengine.com/docs/static/engines/cryengine-5/categories/23756816/pages/25535321
32. Crytek. CRYENGINE 5.6 documentation, "Profiling Quick Guide" (`r_DisplayInfo 1`). *search-recorded* — https://www.cryengine.com/docs/static/engines/cryengine-5/categories/23756816/pages/35260143
33. O3DE. `Code/Framework/AzCore/AzCore/Console/IConsoleTypes.h` (`ConsoleFunctorFlags`). *read* — https://raw.githubusercontent.com/o3de/o3de/development/Code/Framework/AzCore/AzCore/Console/IConsoleTypes.h
34. O3DE documentation. "AZ::Console". *read* — https://www.docs.o3de.org/docs/user-guide/programming/az-console
35. O3DE documentation. "Issue Console Commands from the Settings Registry". *read* — https://docs.o3de.org/docs/user-guide/settings/issue-az-console-commands/
36. O3DE documentation. "CVars" appendix and "Command Line Arguments" (`--name value`). *search-recorded* — https://docs.o3de.org/docs/user-guide/appendix/cvars/ ; https://docs.o3de.org/docs/user-guide/settings/command-line-arguments/
37. RichoDemus. `bevy_console` README. *read* — https://github.com/RichoDemus/bevy-console
38. RichoDemus. `bevy_console` `src/console.rs`. *read* — https://raw.githubusercontent.com/RichoDemus/bevy-console/main/src/console.rs
39. RichoDemus. `bevy_console` `examples/change_console_key.rs`. *read* — https://raw.githubusercontent.com/RichoDemus/bevy-console/main/examples/change_console_key.rs
40. doonv. `bevy_dev_console` README. *read* — https://github.com/doonv/bevy_dev_console
41. martin-t. `cvars` README. *read* — https://github.com/martin-t/cvars
42. CasualX. `cvar` 0.5.0 documentation. *read* — https://docs.rs/cvar/latest/cvar/
43. jakobhellermann. `bevy-inspector-egui` README. *read* — https://github.com/jakobhellermann/bevy-inspector-egui
44. dtolnay. `inventory` documentation. *read* — https://docs.rs/inventory/latest/inventory/
45. dtolnay. `linkme` 0.3.37 documentation. *read* — https://docs.rs/linkme/latest/linkme/
46. Star Citizen Wiki (community). "Console commands". *read* — https://starcitizen.tools/Console_commands
47. Star Citizen Wiki (community). "Guide:Bug reporting". *read* — https://starcitizen.tools/Guide:Bug_reporting
48. Minecraft Wiki (community). "Debug screen". *read* — https://minecraft.wiki/w/Debug_screen
49. Valheim console guides (PC Gamer; WinterNode). *search-recorded* — https://pcgamer.com/valheim-cheats-console-commands ; https://winternode.com/help/games/valheim/administration/enable-valheim-cheats
50. Chet Ramey. GNU Readline Library manual. *read* — https://tiswww.case.edu/php/chet/readline/readline.html
51. fish shell. "Interactive use". *read* — https://fishshell.com/docs/current/interactive.html
52. winit 0.30.12. `KeyEvent`. *read* — https://docs.rs/winit/0.30.12/winit/event/struct.KeyEvent.html
53. winit 0.30.12. `KeyCode`. *read* — https://docs.rs/winit/0.30.12/winit/keyboard/enum.KeyCode.html
54. winit 0.30.12. `Key`. *read* — https://docs.rs/winit/0.30.12/winit/keyboard/enum.Key.html
55. winit 0.30.12. `Ime`. *read* — https://docs.rs/winit/0.30.12/winit/event/enum.Ime.html
56. winit 0.30.12. `Window` (`set_ime_allowed`, `set_ime_cursor_area`). *read* — https://docs.rs/winit/0.30.12/winit/window/struct.Window.html
57. winit v0.30.12 source, `src/platform_impl/windows/keyboard.rs`. *read* — https://raw.githubusercontent.com/rust-windowing/winit/v0.30.12/src/platform_impl/windows/keyboard.rs
58. winit changelog, v0.29 (Servo's documentation mirror). *read* — https://doc.servo.org/winit/changelog/v0_29/index.html
59. W3C. "UI Events KeyboardEvent code Values". *read* — https://www.w3.org/TR/uievents-code/
60. MDN. "Keyboard event code values". *read* — https://developer.mozilla.org/en-US/docs/Web/API/UI_Events/Keyboard_event_code_values
61. SDL. `SDL_Scancode` (SDL2 wiki). *read* — https://wiki.libsdl.org/SDL2/SDL_Scancode
62. kbdlayout.info. French (KBDFR) scancodes. *read* — https://kbdlayout.info/KBDFR/scancodes
63. Wikipedia. "AZERTY". *read* — https://en.wikipedia.org/wiki/AZERTY

## Fetched but unreadable

- Valve Developer Community: "Developer Console Control" and "Developer console". HTTP 403. Seen only through search [14][15].
- Unreal Engine 4.26 "Console Manager" page (`docs.unrealengine.com/4.26/ProductionPipelines/DevelopmentSetup/Tools/ConsoleManager`). HTTP 403. The current page [18] was read instead.
- Epic issue tracker UE-15894. HTTP 403 [24].
- AWS re:Post, "Whats the relation between Cvars and cheats?" (Lumberyard). HTTP 403. Nothing from it is used.
- CRYENGINE `IConsole.h` on GitHub (`CRYTEK/CRYENGINE`, branches `release` and `main`). HTTP 404. The enum was read from CryEngine's own documentation [26].
- Source SDK 2013 at the old `sp/src/...` paths. HTTP 404 after the repository's restructuring. The `src/...` paths were read [10]–[13].
- CRYENGINE 5 "Console Variables & Config Files" [30], CRYENGINE 5 page 29791867, and the CRYENGINE 5.6 "Profiling Quick Guide" [32]. Navigation only, through 256,000 characters of text.
- O3DE "AZ::Console" [34] and "Issue Console Commands" [35]. Navigation only in the first 100,000 characters; the article bodies were read further on.