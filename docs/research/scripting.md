# Scripting: what games use, and what Forge could do

Research for the owner's question of 2026-10-18: "study the use of scripts like Lua or other — what's the game industry using in recent years?" It goes with [data-driven.md](data-driven.md) §3–5 and D-035, which already settled four things: no binary Rust plugins, no native-code mods, data packages first, WebAssembly for mod code later. Behaviour trees and utility AI are in [game-ai-ecosystems.md](game-ai-ecosystems.md). Prediction by digest is D-010, the determinism rules are D-016, and the weather director's scripted commands are D-034. This note does not repeat the sources cited there. Numbers in brackets refer to the source list at the end. Versions are as shown on 2026-10-18. None of this is legal advice.

## Summary

- **What the industry uses, 2020–2026.** There are three camps.
  - **The big engines** pair a compiled language with a visual layer: Unreal has C++ and Blueprints, Unity has C# and its bundled visual scripting, Godot has GDScript, C# and C++. Each is moving its gameplay code to a better-typed language that hot-reloads. Epic says Unreal Engine 6 moves "the gameplay programming model to Verse", with early access targeted for the end of 2027 [1]. Unity's first editor running on CoreCLR, without Mono, reached a 7.0 alpha in October 2026 [8]. Angelscript, a text language Hazelight built into Unreal, has shipped in 11 games [6].
  - **The Lua family** serves designers and mods. Users include Factorio, Noita, Teardown, Project Zomboid, World of Warcraft's interface, Defold and O3DE. Roblox's typed Lua, Luau, is spreading beyond Roblox: Alan Wake 2 replaced a proprietary language with it, and Warframe, Farming Simulator 25 and Second Life use it too [13][17].
  - **Studio languages that are mostly data:** Paradox's script, Bethesda's Papyrus, Bohemia's Enforce Script and Larian's Osiris.
- **WebAssembly is real but niche.** Microsoft Flight Simulator's add-ons, BitCraft's server and Veloren's plugins use it.
- **Visual scripting is not the trend where people write text.** Godot dropped its own at 0.5 % use [11]. Platforms for user-made content keep it as the safe layer: Hytale has "no text-based scripting" and plans sandboxed visual logic [34].
- **Rust options.**
  - `mlua` is the mature binding: Lua 5.1–5.5, LuaJIT and Luau, version 0.12.2 of 3 October 2026 [41].
  - `wasmtime` is the mature sandbox, with monthly releases, long-term-support releases, CPU metering ("fuel") and a specified deterministic mode [43].
  - Rhai and Rune are easy to embed but slow, with small communities.
  - piccolo, Mun, Dyon and Gluon are not production choices.
- **Determinism is where Forge differs from most games.**
  - Lua 5.4 seeds its string hashes from the clock and from memory addresses.
  - Luau hashes object keys by their memory address, and its `math.sin` calls the C library.
  - Both seed `math.random` from the time.
  - Only WebAssembly is deterministic by specification. It has no `sin` instruction, so each module carries its own maths.
  - Forge's clients predict the whole state and compare digests with the server (D-010), so no interpreter should run inside the predicted step.
- **Proposal, for the owner to decide:**
  - keep D-035;
  - write missions as data now, with D-062's first objective;
  - add Luau through `mlua` when hand-written scenario logic or mods need code. It would run on the server and in the interface only, and send tick-stamped commands, as D-034's director does;
  - keep WebAssembly for mod code that must run inside the prediction;
  - no visual scripting yet.
- **Patents.**
  - Two live patents cover particular visual-scripting editors: Amazon's (a model's mesh drawn in the graph) and Epic's (expressions built from stacked code tiles). A text language or a plain node graph uses neither.
  - Microsoft's .NET hot-reload patents do not apply to Forge.
  - A Zynga family on client and server running scripts independently runs to about 2031. Its claims are unread; read them before any mod code runs on both sides.
  - Nothing was found on Lua or WebAssembly sandboxes, instruction budgets, taint or Verse's features. Google Patents throttled the search, so this is not exhaustive.

**Contents**
1. [The survey](#1-the-survey)
2. [Runtimes for a Rust engine](#2-runtimes-for-a-rust-engine)
3. [Patterns](#3-patterns)
4. [Patents](#4-patents)
5. [Licences](#5-licences)
6. [Recommendation](#6-recommendation-a-proposal-for-the-owner)
7. [Checked and left out](#7-checked-and-left-out)
8. [Verification notes](#8-verification-notes)
9. [Sources](#9-sources)

---

## 1. The survey

| Engine or studio | Language | Kind | Since, and the trend | Sources |
|---|---|---|---|---|
| Unreal Engine 5 (Epic) | C++ and Blueprints | native code plus a visual graph run as bytecode | Blueprint nativization (conversion to C++) was removed in UE 5.0; Epic warns that "performance may be impacted" | [3] |
| Unreal Engine 6 (Epic) | Verse, with C++ underneath | text: functional-logic, with effects, transactions and structured concurrency | Epic: "We're moving the gameplay programming model to Verse, which transactionalizes C++"; core systems are "landing as Verse-scriptable components" of the Scene Graph; early access targeted for the end of 2027 | [1][2] |
| UEFN / Fortnite (Epic) | Verse | text | since UEFN in 2023; persistence, and compatibility rules enforced at publication (data-driven.md §3) | [4][5] |
| Hazelight, Embark, Croteam and others on Unreal | Angelscript (UnrealEngine-Angelscript) | text, static types, hot reload | used by Hazelight since 2018; 11 shipped games, among them It Takes Two, Split Fiction, THE FINALS, ARC Raiders, Titan Quest II and the Gothic 1 Remake; Split Fiction has over 1.7 million lines in more than 16,000 files | [6] |
| Tencent (Unreal and Unity games) | Lua through UnLua and xLua | text | xLua's headline feature is patching shipped C# methods with Lua | [7] |
| Unity | C# (Mono and IL2CPP), Burst's "High-Performance C#" subset, Visual Scripting (formerly Bolt) | native-like text plus a graph | Visual Scripting installed by default since 2021.1; Unity 6.7 LTS has an experimental CoreCLR player; 7.0 alpha (October 2026) is the first CoreCLR editor without Mono, and domain reloads "are gone in Unity 7" | [8][9][10] |
| Godot | GDScript, C#, C and C++ through GDExtension | text | VisualScript removed in 4.0 (0.5 % of more than 5,000 poll respondents used it); C# projects in Godot 4 cannot be exported to the web | [11][12] |
| Roblox | Luau | text, gradual types | types released 2020; open source (MIT) 2021; native code generation in a Studio beta in 2023, Android in 2025; new type solver released generally in November 2025; Parallel Luau actors | [13][15][16] |
| Remedy, Northlight (Alan Wake 2, 2023) | Luau | text | "We switched our scripting language from a proprietary language into Luau"; used for level scripting and gameplay systems, with live editing | [17] |
| Warframe, Farming Simulator 2025, Second Life | Luau | text | listed in Luau's README | [13] |
| O3DE | Lua and Script Canvas; Python for the editor | text plus a graph | inherited from Lumberyard | [18] |
| CryEngine | Lua, Flowgraph, Schematyc, C# | text plus two graphs | Schematyc arrived in 5.3 (2016) | [19] |
| Defold | Lua 5.1 on LuaJIT; plain Lua 5.1.4 on HTML5 | text | JIT off on iOS and Switch; "ONLY use language features from Lua 5.1" | [20] |
| Bethesda, Creation Engine 2 (Starfield) | Papyrus | text, compiled, event-driven | kept for Starfield; Creation Kit of June 2024 with a VS Code integration for Papyrus | [21] |
| Bohemia Interactive, Enfusion (DayZ, Arma Reforger) | Enforce Script | text, C#-like, inspired by C++ | `modded class` (with `vanilla` and `super`) and sealed classes; data mods by override, inherit, duplicate and replace; "not all game features are scriptable" | [22] |
| Paradox, Clausewitz/Jomini (CK3 and successors) | Paradox script | a declarative language of triggers, effects and scopes | loops capped at 1,000 iterations; no OS access, no string manipulation, no inline maths; AI mostly in C++ | [23] |
| Larian (Baldur's Gate 3) | Osiris; Lua through the community Script Extender | rules and databases ("events" in, "calls" out) | Norbyte's extender "adds Lua/Osiris scripting support"; v32 in June 2026 | [24] |
| Factorio | Lua 5.2.1, modified | text | determinism retrofitted (no `coroutine`, `pairs` in insertion order, custom trigonometry); `storage` holds the only saved state | [25], data-driven.md §4 |
| Teardown | Lua | text | `init`, `tick` (once a frame), `update` (fixed 60 Hz, at most twice a frame), `draw` (interface only) | [26] |
| Noita | Lua 5.1 on LuaJIT | text | restricted mods by default; "unsafe" mods (input and output) need a setting and cannot go on the Workshop | [27] |
| Project Zomboid | Lua through Kahlua2 (Java, MIT) | text | much of the game's own logic is Lua, in client, server and shared folders that mods add to and override | [28] |
| World of Warcraft | a subset of Lua 5.1 | text, interface only | secure execution and taint since patch 2.0 (2006), to stop add-ons automating combat | [29] |
| s&box (Facepunch); Space Station 14 | C# | text, hot-loaded | s&box compiles changes "within milliseconds" and has an API whitelist; SS14 checks content for verifiable IL and against a whitelist of members | [30][31] |
| Minecraft Bedrock | JavaScript (QuickJS-based), ES modules | text | the Script API in behaviour packs; breaking changes are frequent | [33] |
| Hytale (early access January 2026) | Java plugins, JSON data, planned visual scripting | server-side only | "We do not have text-based scripting and we do not intend to add it"; it calls Lua a "false compromise" | [34] |
| Microsoft Flight Simulator | WebAssembly | native code compiled ahead of time | add-ons on PC and Xbox | data-driven.md §3 |
| BitCraft Online (Clockwork Labs) | Rust as a SpacetimeDB module | WebAssembly | the server is "structured as a SpacetimeDB module", and SpacetimeDB compiles Rust and C# modules to WebAssembly | [35] |
| Veloren; Ambient | Rust compiled to WebAssembly | plugins | Veloren: experimental API (data-driven.md §3); Ambient, a WebAssembly-first Rust engine, is "paused indefinitely" | [36] |
| Bevy | Rust (`subsecond` hot patches); community `bevy_mod_scripting` (Lua, Luau, Rhai through reflection) | — | no official scripting; the plugin calls itself work in progress and does not run on WebAssembly | [37] |
| Flecs | Flecs Script | declarative markup for entities | "a runtime interpreted DSL for creating entities and components"; scripts re-run when their inputs change | [38] |

**How it changed:**
- **One gameplay language, typed and hot-reloaded.**
  - Verse becomes Unreal's gameplay language [1].
  - Angelscript sits "between" Blueprints and C++, hot-reloads, and is turned into C++ for shipping, where it "approaches native C++ performance" [6].
  - Unity's move to CoreCLR replaces domain reloads with reloads through `AssemblyLoadContext` [8].
  - Luau added gradual types and a new type solver [13][15]. Godot recommends static typing in GDScript.
- **Studios drop their own languages.** Remedy moved to Luau [17], and CD Projekt Red moved from REDengine to Unreal Engine 5 in 2022 [55]. Large catalogues keep theirs: Bethesda, Paradox, Bohemia and Larian.
- **Lua is still how indie and simulation games are modded.** Luau is the version gaining studio adopters [13].
- **Visual scripting splits by audience.**
  - It stays where non-programmers build: Blueprints, and Hytale's plan [34].
  - It disappears where users read text: Godot removed its own [11].
  - The reports on Blueprints in UE6 differ. Inven says "actors and Blueprints will also gradually shift to the new framework" [2]. Epic's own summary does not mention Blueprints [1]. Treat it as unconfirmed.
- **WebAssembly is where code must be untrusted, portable or server-hosted:** MSFS, BitCraft, Veloren. It is not yet a common choice for designers' scripts.
- **Untrusted code is being pushed onto servers.** Roblox makes the server the authority (data-driven.md §4). Hytale has "no client mods" [34]. WoW's taint system keeps add-ons away from decisions [29].

**A caution on Verse.** In December 2022 Epic promised an open-source compiler, verifier and runtime. No release was found, so Verse is a design reference for Forge, not a dependency. The ICFP 2023 paper defines its core calculus [5].

---

## 2. Runtimes for a Rust engine

### The candidates

| Runtime | Rust crate | Execution | Sandbox and limits | Determinism | Hot reload | Tooling | Licence | Maintainer and health |
|---|---|---|---|---|---|---|---|---|
| **Lua 5.4 / 5.5** (PUC-Rio) | `mlua` 0.12.2 (2026-10-03), 2.9k stars; `rlua` archived 2025-09-12 as "a thin transitional wrapper" over mlua [41] | bytecode interpreter | no sandbox mode: build the environment yourself. `Lua::new_with` loads a safe subset of the libraries, `set_memory_limit`, `set_hook` every N instructions [41] | 5.4 seeds string hashes from `time(NULL)` and three addresses (ASLR) [39]; 5.5's `lua_newstate` takes the seed as an argument [39]; "The order in which the indices are enumerated is not specified" [39]; `math` uses the C library; `math.random` is seeded "with a weak attempt for randomness" [39] | reload the chunk | LuaLS (5.1–5.5 and LuaJIT, "nearly a million installs") [50]; Teal, a typed dialect for 5.1–5.4 [50] | MIT | PUC-Rio; 5.5.0 released 22 December 2025 [39] |
| **LuaJIT 2.1** | `mlua` (`luajit`) | tracing JIT and a fast interpreter | as Lua 5.1 | as Lua; not examined further | reload the chunk | LuaLS | MIT | "rolling releases", no tarballs [40]; "the JIT compiler is disabled for iOS" and on consoles "only the fast interpreter is built" [40] |
| **Luau** (Roblox) | `mlua` (`luau`, `luau-jit`, `luau-vector4`) [41] | interpreter tuned to match LuaJIT's on some workloads; optional native code for x64 and arm64, opt-in per function or module [13] | built-in sandbox: `Lua::sandbox` (read-only libraries and globals), an interrupt callback (`set_interrupt`), no bytecode loading, `set_memory_limit` [13][41] | string hashes unseeded, from the length and content only [14]; object keys hash by address [14]; `math.sin` calls the C library's `sin` [14]; `math.random` is PCG32 seeded from a pointer, `time` and `clock` unless `math.randomseed` is called [14]; weak tables may drop keys during iteration [13] | reload the module | gradual types; `luau-lsp` (MIT) with definition files for custom hosts and an `analyze` command for CI [50] | MIT | Roblox's team; regular releases; adopted outside Roblox [13] |
| **piccolo** | 0.3.3 (2024-06) [42] | stackless VM in pure Rust | fuel per `step`, memory counted in the GC arena | not examined | — | no debugger | MIT or CC0 | one author; "still very experimental", slower than PUC-Rio Lua, no `__gc`, no weak tables [42] |
| **WebAssembly: wasmtime** | 49.0.3 (2026-10-14); a major version every month, every 12th supported for 24 months, security fixes guaranteed [43] | Cranelift, at load or ahead of time; Pulley, a portable interpreter, where Cranelift does not run [43] | the module reaches the host only through its imports; fuel (deterministic) or epochs ("up to 2-3x" faster, not deterministic); a `ResourceLimiter` [43] | specified: the Wasm 3.0 profile, NaN canonicalisation, deterministic relaxed SIMD (data-driven.md §3, [43]); floating point has only add, sub, mul, div, sqrt, min, max, rounding and sign operations, so a guest's `sin` is compiled into the module [44] | re-instantiate; carry state by copying memory or by serialisation | the guest language's own tools; native debuggers with debug info [43] | Apache-2.0 WITH LLVM-exception [43] | Bytecode Alliance |
| **wasmer** | 7.5.0 (2026-10-01) [45] | Singlepass, Cranelift, LLVM; "headless" precompiled | not read | as Wasm | as Wasm | — | MIT | Wasmer Inc. |
| **wasmi** | 2.0.0 (2026-09-01) [45] | interpreter, `no_std` | fuel | "deterministic" (its own claim) | as Wasm | — | MIT or Apache-2.0 | audited 2023 and 2024; used by Soroban, Typst, Zellij [45] |
| **Rhai** | 1.26.1 (2026-09-10) [46] | walks the syntax tree | operation, depth and size limits (data-driven.md §3) | maps are `BTreeMap`, though `keys` is documented as "random order"; `no_float` and `decimal` features [46] | recompile | `debugging` feature [46] | MIT or Apache-2.0 | small team; "roughly 2-3x slower than Python 3" [46] |
| **Rune** | 0.14.2 (2026-05-22) [47] | stack VM, async, reference counting | `runtime::budget` [47] | not examined | listed as a feature | not confirmed | MIT or Apache-2.0 | one main author |
| **Mun** | `mun_runtime` 0.4.0 (2022-12-12) [48] | compiled ahead of time with LLVM, static types | — | — | "first class hot-reloading" | — | MIT or Apache-2.0 | a few commits in 2024–2026, no release since 2022 |
| **Koto, Dyon, Gluon** | Koto 0.16.1 (2026-01-05); Dyon and Gluon committed to in 2026 [48] | interpreters | — | — | — | — | Koto's licence not confirmed | niche |
| **Starlark** | `starlark-rust` (Meta) [49] | interpreter | hermetic by design | "a deterministic language inspired by Python3", made for build configuration | — | — | Apache-2.0 | no promise of API stability |
| **JavaScript** | `rquickjs` 0.14.0 (QuickJS-NG, MIT); `boa_engine` 0.22.0 ("experimental") [49] | interpreter | QuickJS has memory and interrupt limits (not read here) | as Lua (GC, maths) | reload the module | TypeScript tooling | MIT; boa MIT or Unlicense | Minecraft Bedrock is the game precedent [33] |
| **C# / .NET** | `netcorehost` 0.22.0 (MIT) [49] | CoreCLR, JIT | **none in-process**: Code Access Security "is not supported in .NET Core, .NET 5, or later" [32]; SS14 whitelists IL instead [31] | GC; the JIT's floating point not examined | `AssemblyLoadContext` | excellent | MIT | Microsoft |

### Performance
- **No JIT on consoles or iOS.**
  - LuaJIT's JIT is off on consoles and iOS [40]. Defold says the same for iOS and Switch [20].
  - Luau puts its effort into the interpreter, and its JIT is optional and opt-in [13].
  - WebAssembly has two ways round the ban: compile ahead of time (MSFS on Xbox, data-driven.md §3) or interpret with Pulley or wasmi [43][45].
- **Interpreters differ by an order of magnitude.**
  - Luau is "noticeably faster than Lua 5.x" and on some workloads matches LuaJIT's interpreter, which is written in assembly [13].
  - Rhai is 2–3 times slower than Python 3 by its own benchmarks [46].
  - piccolo is "not yet competitive" with PUC-Rio Lua [42].
- **Nothing here was measured on Forge.** The proposal's first demo should time a mission tick in each candidate.

### Sandboxing (the checklist)
- **Luau's sandbox** is the reference checklist: `io` and `package` removed; read-only libraries and globals; a separate environment per script; no loading of untrusted bytecode; an interrupt to stop runaway scripts; thread identities to guard privileged APIs (data-driven.md §3, [13]). `mlua` exposes it as `Lua::sandbox`, `set_interrupt` and `set_memory_limit` [41].
- **Plain Lua 5.4 and 5.5** need an environment built by hand plus an instruction hook. `Lua::unsafe_new` "will allow to load C modules" and must never run mods [41].
- **WebAssembly is sandboxed by construction.** A mod can call only what it imports. Fuel limits CPU deterministically, and the resource limiter limits memory [43].
- **Two trust tiers are common.** Noita has restricted and "unsafe" mods [27]. WoW separates its own secure code from tainted add-on code [29]. Luau has thread identities [13]. Hytale gives Java plugins to server owners and keeps sandboxed visual logic for sharing [34].
- **In-process .NET cannot sandbox** [32]. Mods in C# mean either verifying IL against a whitelist, as SS14 does [31], or a separate process.

### Determinism
- **Run-to-run reproducibility on one binary** is what replays and Forge's bit-exact captures need.
  - **Luau** gives it with three rules:
    - call `math.randomseed` from `Seed::derive`;
    - never let simulation logic depend on `pairs` order over tables keyed by objects, because those keys hash by address [14];
    - no weak tables in simulation scripts [13].
  - **Lua 5.4** does not give it as built: its string-hash seed changes with each run [39]. Lua 5.5 lets the host fix the seed [39]. Whether `mlua` exposes that seed was not checked.
- **Same results on every machine and compiler** is what the predicted simulation needs (D-016).
  - In **Lua and Luau** it would mean, as in Factorio:
    - replacing `math` with `dmath`;
    - ordered iteration;
    - checking the interpreter's floating point across compilers. #281 found fused multiply-adds in shaders; the C++ VM was not examined.
  - **WebAssembly** gives it by specification once NaN canonicalisation and deterministic relaxed SIMD are switched on and the limits are fuel and memory limits [43][44].
- **Snapshots and rollback.** Forge's client goes back to the server's state when digests differ (D-010), so any script state inside the simulation must be in the snapshot.
  - A WebAssembly instance's state is its linear memory, globals and tables, which can be copied.
  - A Lua heap is a graph of closures and coroutines with no built-in serialisation. Factorio removed `coroutine` and saves only `storage`: "Functions are not allowed in `storage`" [25].
  - The cheapest rule for any language: scripts keep their persistent state in Forge records or components, and the VM's own tables are caches.

### Memory and the frame budget
- **Luau's collector** is incremental and paced by a "proportional–integral–derivative controller", with a paged sweeper; values take 16 bytes [13].
- **Lua 5.5** makes "major garbage collections done incrementally" [39].
- **`mlua`'s `gc_step`** lets the engine place collection work at a fixed point of the frame [41].
- **Others.** WebAssembly guests manage their own linear memory, with no host collector unless they use Wasm GC. Rune counts references [47].

### Hot reload
- **Lua and Luau** re-run the changed chunk and rebind its functions. State survives if it lives in records (above). Remedy calls live editing a reason for Luau [17].
- **WebAssembly** re-instantiates the module and restores state from records or a serialised blob.
- **Rust** keeps `subsecond` for system bodies in development (D-035).

### Tooling
- **Luau has the best editor story among the embeddable languages.**
  - Gradual types, and a new solver released generally in November 2025 [15].
  - `luau-lsp` with definition files that Forge could generate from its API, and a command-line check for CI [50].
  - No standard debugger outside Roblox Studio was found.
- **Lua 5.x** has LuaLS with annotations [50], and Teal for types [50].
- **WebAssembly** inherits the guest language's tools (Rust's compiler, tests and clippy) and native debuggers [43]. It is the best tooling for programmers and the heaviest for designers.

### C#/.NET from Rust
Hosting CoreCLR is possible (`netcorehost`), but three things count against it:
- it adds a garbage-collected runtime;
- it cannot sandbox mods [32];
- it would need IL2CPP-like ahead-of-time work for any platform that forbids JIT.

It is realistic only if C# gameplay code for trusted programmers were the goal, the s&box model [30]. That is not Forge's case.

---

## 3. Patterns

**What scripts may touch:**
- **Events in, calls out.**
  - Factorio's `script.on_event` and `on_nth_tick` [25].
  - Teardown's four callbacks [26].
  - Osiris: "events" from the game, "calls" back [24].
  - WoW's frames and events [29].
- **Components through reflection.**
  - `bevy_mod_scripting` binds every reflected type [37].
  - UnLua binds Unreal's reflected classes, properties and functions [7].
  - In UE6, core systems become "Verse-scriptable components" [1].
- **Coroutines for missions.**
  - Lua's coroutines let a script wait inside nested calls (data-driven.md §3).
  - Verse structures them. `sync` waits for all arms, in source order. `race` takes the first and cancels the rest. `rush` takes the first and lets the others run. `branch` ends with its scope. `spawn` can outlive its creator. Cancellation is cooperative, at suspension points [4].
  - The catch: a suspended coroutine cannot be saved. Factorio removed the library for that reason [25]. Missions that are saved mid-way keep their stage in data.
- **Commands, not writes.**
  - Roblox clients ask and the server decides (data-driven.md §4).
  - Parallel Luau "scripts running in parallel generally can't write to the data model" [16].
  - D-034 already has the Forge form: a director's command "travel[s] in the replicated event stream like any event, so every machine evaluates the same".

**Data instead of code:**
- **Paradox:** triggers, effects and scopes, with a loop cap and no OS [23].
- **Enfusion:** data mods by override, inherit, duplicate and replace [22].
- **Hytale:** NPC "instruction lists" in JSON over elements written in Java [34].
- **Flecs Script:** declarative scenes and templates [38].
- **Unreal's StateTree:** "a general-purpose hierarchical state machine" [52].
- **Behaviour trees and utility AI:** already chosen in game-ai-ecosystems.md.
- **Dialogue in ink:** MIT [51]. `blade-ink-rs` (Apache-2.0) claims full compatibility. `YarnSpinner-Rust` is work in progress and tied to Bevy [51].
- **The common shape:** native code writes the elements (conditions, actions); data composes them; a script language is kept for what composition cannot say.

**Keeping scripts off the critical path:**
- **Orchestrate, don't iterate.** Godot notes that "most engine work runs in fast C++ code" [12]. Per-entity, per-frame loops belong in native systems.
- **Run less often.** Teardown runs `update` at a fixed rate and `tick` per frame [26]. Factorio has `on_nth_tick` [25].
- **Compile the hot path.**
  - Unreal removed nativization and accepts the cost [3].
  - Angelscript transpiles for shipping [6].
  - Unity compiles a C# subset with Burst [10].
  - Luau's native code is opt-in per module [13].
- **Parallelise reads, serialise writes.** Parallel Luau's actors and its serial phase [16].
- **Budgets.** Instruction hooks, interrupts or fuel per tick [41][43]. A profiler zone per script, which in Forge would mean the F1 overlay.

---

## 4. Patents

Searches were run on 2026-10-18 through Google Patents' query endpoint (Roblox's and Epic's portfolios, and visual scripting) and through web search (hot reload, sandboxed game scripts, rollback). "Claims read" means claim 1 was read. Google Patents started refusing requests (HTTP 503) after four queries, so the rest come from titles, abstracts and search snippets.

| Patent | Holder | Dates, status | What it covers | Does Forge touch it? |
|---|---|---|---|---|
| **US 10,909,754 B1** "Visual scripting for multi-dimensional elements" | Amazon Technologies | priority and filing 2017-08-18; granted 2021-02-02; active; expires about 2039-05-09 (Google's adjusted date) | **Claims read:** a visual scripting system determines a 3D model and its bounding box, draws a 2D mesh of the element in the scripting interface, takes an action instruction, and performs it in play when a criterion on part of that mesh is met | No, for a text language or an ordinary node graph. Only an editor that draws a model inside the graph and binds actions to its parts would come near it: avoid that design, or read the dependent claims first |
| **US 11,144,284 B1** "Method and system for code tile programming" | Epic Games | priority 2019-02-01; granted 2021-10-12; active; about 2039-06-06 | **Claims read:** draggable code-tile fragments placed on three superimposed layers generate an object-oriented expression of operators and operands | No, unless Forge builds an expression editor of stacked tiles |
| **US 11,755,313 B2** "Implementing changes made to source code of reloadable types at runtime" (WO2023287574, EP4371005); **US 11,847,433 B2** "Source code editing combining edit and continue with hot reload" | Microsoft Technology Licensing | '313: priority 2021-07-12, granted, expiry about 2041 (term adjustment not read); '433: dates not read | .NET Hot Reload of types (titles and abstracts) | No. Forge hosts no .NET. Reloading a Lua chunk and `subsecond`'s function patching are different mechanisms |
| **US 8,449,397 B2**, **8,449,398 B2**, **8,562,441 B2** "Secure, parallel, and independent script execution" | Zynga | priority 2011-05-03 (the '397 is a continuation); '397 granted 2013-05-28; the family expires about 2031 (adjustments not read) | a client and a game server independently run a virtual game's scripts in parallel (descriptions; claims not read) | The proposal runs scripts on the server only, which is outside that description. **Before any design where the same mod code runs independently on client and server** (WebAssembly in the predicted step), read the claims and ask the owner. Client prediction long predates 2011, but that prior art was not analysed here |
| **US 11,579,866 B2** "Developer collaboration control system" | Roblox | priority 2018-12-20; granted 2023-02-14 | collaborative editing of code, with interpretation and compilation of changes (snippet) | Only with a live script editor for several users; claims not read |

**Checked, not relevant:**
- Epic US 8,819,641 B1 "Program state reversing software development tool": priority 2006; a reversible debugger.
- Take-Two US 11,027,196 B2 and US 11,992,755 B2: routing players to a game server dedicated to a "sandbox application". This is server routing, not code sandboxing.
- SunEdison/GlobalWafers US 9,459,935 B2, "deterministic and multithreaded script objects and script engine": priority 2012-12-28, for semiconductor tooling.
- Citrix US 9,639,432 B2 and US 10,379,967 B2 "Live rollback for a computing environment": a virtual machine restored from streams of snapshot, memory and I/O entries.
- Kabam/EA US 9,174,118 B1: anti-cheat that detects client modification through script injection.

**Nothing found on:**
- Lua or Luau sandboxing, instruction budgets or fuel;
- WoW-style taint;
- Blueprints;
- Angelscript's transpiling;
- Verse's failure, effects, transactions or structured concurrency. Epic's 15 results for "programming language", "script", "compiler", "transactional" and "speculative" included none of these.

**WebAssembly itself.**
- The W3C WebAssembly Working Group page says "No patent disclosures have been made for any specifications of this group", and its participants make royalty-free licensing commitments by joining [44].
- Wasmtime's Apache-2.0 licence grants a patent licence from each contributor for their contributions (§3).
- Lua and Luau are MIT, which has no explicit patent grant. Their techniques are decades old.

---

## 5. Licences

| Component | Licence | Note |
|---|---|---|
| Lua 5.4/5.5, LuaJIT, Luau | MIT | [39][13]; LuaJIT's from its site |
| `mlua` | MIT | vendors the C or C++ sources and builds them with the system compiler (MSVC here) [41] |
| `wasmtime` | Apache-2.0 WITH LLVM-exception | [43] |
| `wasmer` / `wasmi` / Extism | MIT / MIT or Apache-2.0 / BSD-3-Clause | [45] |
| Rhai, Rune, Mun, `bevy_mod_scripting` | MIT or Apache-2.0 | [46][47][48][37] |
| piccolo | MIT or CC0 | [42] |
| `starlark-rust` | Apache-2.0 | [49] |
| `rquickjs` / `boa_engine` / `netcorehost` | MIT / MIT or Unlicense / MIT | [49] |
| Kahlua2 (reference only) | MIT | [28] |
| LuaLS, Teal, `luau-lsp` | MIT | developer tools, not shipped [50] |
| ink / `blade-ink-rs` / `YarnSpinner-Rust` | MIT / Apache-2.0 / MIT or Apache-2.0 | [51] |
| UnLua, xLua | MIT | references [7] |
| UnrealEngine-Angelscript | not read | needs Unreal's source licence anyway [6] |

**Fit with the owner's rules.**
- Everything above is free and royalty-free.
- A runtime that is brought in gets its line in `CREDITS.md`, and `cargo run -p credits` updates the crate list.
- "Giving back" would apply to the language's owners: Roblox's Luau, PUC-Rio's Lua, the Bytecode Alliance.
- **Forge's `unsafe` rule holds.** `mlua` ("a huge amount of unsafe code" [41]) and `wasmtime` keep their `unsafe` inside the dependency, and a `forge-script` crate would use only their safe APIs.

---

## 6. Recommendation (a proposal for the owner)

**Why Forge is not a typical case.**
- Forge's clients predict the entire simulation and compare its digest with the server's (D-010).
- Its captures are bit-exact (D-017).
- Its generator will write the missions for generated maps (D-062). Generated content is data, not scripts.

Most scripting in the survey runs where none of this applies. So: Rust and data first; a script language at the edges; a deterministic sandbox only where code must enter the prediction.

**P1. The engine and the predicted simulation stay in Rust** (as D-035). This covers movement, weapons, physics and the AI core. Iteration comes from data hot reload, with `subsecond` in development.

**P2. Missions as data, now, with D-062's first objective.**
- **The record:** a `Mission` record in `forge-data` packages, with:
  - objectives;
  - triggers: a volume entered, an item picked, an entity killed, a timer, all-of and any-of;
  - steps, combined by four Verse-like operators: in sequence, all (`sync`), first (`race`) and in the background (`branch`) [4].
- **How it runs:** a Rust system evaluates it at the fixed tick, so it is digested, saved by stage, and hot-reloaded. The generator writes these records for generated maps.
- **Around it:** dialogue later in ink (`blade-ink-rs`); AI stays behaviour trees and utility.
- **Keep it small.** Nystrom's warning applies (data-driven.md §3): stop at what generated missions need, and send the rest to P3.

**P3. When code is needed, use Luau through `mlua`, in a `forge-script` crate.** "Needed" means hand-written scenario logic, D-034's "scripts designers and players write for scenarios, cinematics and ambiance", interface logic, or mods.
- **Where it runs:** on the server and in the client's interface, never inside the predicted step.
- **What it does:**
  - its effects are commands stamped with a tick. They travel in the event stream as D-034's director commands do, and Rust systems on every machine apply them;
  - it reads through a query API by stable ids, and writes only commands.
- **Where it keeps state:** in records, as Factorio's `storage` [25]. Saves, snapshots and hot reload then work, and VM globals are caches. Coroutines are allowed only for what may restart from a checkpoint (cinematics).
- **Sandbox:**
  - `Lua::sandbox(true)` and only Forge's libraries;
  - `set_memory_limit`, and a per-tick budget through `set_interrupt` [41];
  - one VM per mod or per trust tier;
  - no bytecode loading [13].
- **Reproducible for replays and captures:**
  - `math.randomseed` from `Seed::derive`;
  - `math` mapped to `dmath`;
  - clocks removed;
  - ordered-iteration helpers;
  - no weak tables [13][14].
- **Tooling:**
  - definition files generated from the Rust API for `luau-lsp`;
  - `luau-lsp analyze` in the gate [50];
  - an F1 zone per script.
- **Why Luau rather than Lua 5.5:**
  - its sandbox is built in;
  - its string hashing is unseeded;
  - it has types and a language server;
  - it is fast without a JIT, which matters for consoles later;
  - Roblox maintains it, and studios are adopting it [13][17].
- **What Luau gives up:**
  - Lua 5.3+'s 64-bit integers. As of the 2025 recap Luau has no integer type, so ids travel as opaque handles [13];
  - the Lua 5.x modding culture of Project Zomboid and Factorio (option C).

**P4. WebAssembly components (wasmtime) stay D-035's route for code that must enter the predicted simulation**, or that comes in other languages, when a real case appears (a mod that adds a weapon behaviour). Only WebAssembly is deterministic by specification and snapshots by copying memory [43][44]. Before building it:
- read the Zynga family's claims (§4);
- measure one idea, Forge's own and unmeasured: Luau's VM compiled to WebAssembly would give the same language with WebAssembly's determinism.

**P5. No visual scripting for now.**
- The reasons: Godot's 0.5 % [11]; the cost of an editor; and Forge's authors write text.
- A graph view of P2's mission records, when the editor comes, is cheaper than a visual language.
- Steer clear of stacked-tile expression editors (Epic) and of model meshes bound inside the graph (Amazon) (§4).

### The alternatives and their costs

| Option | Gives | Costs | Right when |
|---|---|---|---|
| A. Rust and data only, no scripting ever | the simplest; deterministic; the fastest | no code mods; every logic change is a rebuild (`subsecond` only in development); modders need Forge's source | mods stay data-only (D-035's first step) and the authors stay programmers |
| **B. Luau through `mlua` (P3, proposed)** | sandbox, types, LSP, fast interpreter, an industry path | a C++ build in the dependency; bindings and definition files; a reproducible profile to keep; not usable in the prediction | scenario scripts, interface and mods, with the server as authority |
| C. Lua 5.5 through `mlua` | standard Lua, integers, a fixed hash seed [39], the culture of Project Zomboid and Factorio | sandbox and environment by hand; types only by annotation (LuaLS) or Teal | modders' familiarity matters more than types |
| D. WebAssembly only (wasmtime and WIT) | one sandbox for everything; deterministic; any language; snapshots by copy | modders need a compiler; the API in WIT; the largest dependency; designers compile before they see a change | mod code must run inside the prediction |
| E. Rhai or Rune | pure Rust, simple embedding | 2–3 times slower than Python (Rhai) [46]; small communities; weak tooling | developer tools and test scenarios, not mods |
| F. C# through `netcorehost` | a familiar language | a garbage-collected runtime, no sandboxing [32], JIT bans | trusted C# programmers only; not proposed |
| G. JavaScript through `rquickjs` | TypeScript tooling; Minecraft Bedrock's precedent [33] | garbage collection; determinism as for Lua; the engine changes fast | the modders targeted know TypeScript better than Lua |

### What the owner would decide
1. Missions as data now (P2)?
2. The script language when it is needed: Luau (proposed), Lua 5.5, JavaScript, or none?
3. Scripts never inside the predicted step: accept (P3)?
4. Mods: server-first, as Hytale and Roblox do (proposed), or full access on client and server, as Project Zomboid's client, server and shared folders do?
5. No visual scripting until the editor (P5)?

**Nothing in P1–P3 builds on a live patent found here.** P4 waits for a reading of Zynga's claims.

### The demo that would prove P2 and P3
- **The objective:** the village map's objective (reach a room, find an item) written once as a mission record and once as a Luau script that sends the same commands.
- **What must match:** both runs give the same digests and pixel-identical captures.
- **The budget:** a script that loops forever is stopped by the budget and reported, with its F1 zone shown.
- **Hot reload:** a script edited while the map runs takes effect without losing the mission's stage.
- **The numbers to report:** the script's time per tick, its memory, and the time to reload it.

---

## 7. Checked and left out

- **Unreal's "Blueprint vs C++" page:** only its navigation rendered, through the reader proxy. The UE5 migration guide's wording on nativization comes from a search snippet [3].
- **Blueprints in UE6:** "deprecated later, with conversion tools" appeared only in secondary reports (Outlook Respawn, Strafe). It is not claimed. Inven's sentence is quoted instead [2].
- **Verse's open-source compiler:** promised in December 2022 (Haskell eXchange slides, via search). No release was found.
- **Bohemia's community wiki:** served a bot check, which was not bypassed. The Enforce Script description comes from the Reforger news post [22] and search snippets.
- **Project Zomboid's 2012 modding post:** refused with a 403. Kahlua2's repository is the source used [28].
- **The Starfield Creation Kit article:** refused with a 403. Search snippets of the same article are used [21].
- **Embark's Ark:** its `ark-api` crates for WebAssembly modules exist (search). Their status is unknown, and nothing is claimed beyond that.
- **Patents left open:**
  - Unity's portfolio and a general visual-scripting query were not run (503s);
  - the claims of the Microsoft, Zynga and Roblox patents were not read;
  - US 2024/0390801 A1, cited secondhand as a game server sandboxing event-driven code, could not be opened.
- **Lua and Luau debuggers:** not evaluated.
- **QuickJS's limits:** not read.
- **wasmer's metering:** not read.
- **Koto's licence:** not confirmed.
- **Balatro, Garry's Mod, Stormworks, Tabletop Simulator, Beyond All Reason:** not researched. The Lua examples above cover the same ground.

## 8. Verification notes

- **Method.** WebSearch and WebFetch only. No browser pane or browser automation was opened.
  - Unreal's pages were read through the public reader proxy `r.jina.ai`.
  - Luau's VM behaviour (hash seeds, pointer hashing, `math`) was read from its source files on GitHub [14].
  - Lua 5.4's seed was read from `lstate.c` [39].
- **Text aimed at AI tools.** The Space Station 14 sandboxing page contained text addressed to summarisers. It was ignored.
- **One stored file.** One USPTO PDF was image-only. The fetch tool stored a copy in the session's tool-results folder. Nothing was opened or run.
- **Fetch-tool summaries.** These were cross-checked where wording matters: quotes for Epic, Hytale, Remedy, Godot, Unity, LuaJIT and Lua.
- **Limits.** Google Patents returned 503 after four queries. Claims were read for two patents only.
- **Versions** are as displayed on 2026-10-18 and will drift.
- **Forge's own judgement, not from sources:**
  - the proposals;
  - the "Luau VM in WebAssembly" idea;
  - the rule that scripts keep state in records;
  - the costs given as relative.

## 9. Sources

1. Epic Games, "State of Unreal 2026: Top news from the show", 17 June 2026 — https://www.unrealengine.com/news/state-of-unreal-2026-top-news-from-the-show
2. S. Yoon, "Unreal Engine 6 to Feature Fundamental Overhaul, Targeting Early Access Release by Late 2027", Inven Global, 17 June 2026 — https://www.invenglobal.com/articles/22900/unreal-engine-6-to-feature-fundamental-overhaul-targeting-early-access-release-by-late-2027
3. Epic Games, "Unreal Engine 5 Migration Guide" (search snippet) — https://dev.epicgames.com/documentation/unreal-engine/unreal-engine-5-migration-guide
4. Epic Games, *The Book of Verse*, "Concurrency" — https://github.com/verselang/book/blob/main/docs/14_concurrency.md — https://verselang.github.io/book/
5. L. Augustsson et al., "The Verse Calculus: a Core Calculus for Deterministic Functional Logic Programming", ICFP 2023, doi:10.1145/3607845 — https://www.cs.tufts.edu/~nr/cs257/archive/simon-peyton-jones/verse-icfp23.pdf
6. Hazelight, Unreal Engine Angelscript: home, resources, development status — https://angelscript.hazelight.se/ — https://angelscript.hazelight.se/project/resources/ — https://angelscript.hazelight.se/project/development-status/
7. Tencent, UnLua — https://github.com/Tencent/UnLua — xLua — https://github.com/Tencent/xLua
8. Unity, "CoreCLR, Scripting, and .NET Update, October 2026", 5 Oct 2026 — https://discussions.unity.com/t/coreclr-scripting-and-net-update-october-2026/1738338
9. Unity Manual 2021.1, "Visual Scripting" (search snippet) — https://docs.unity3d.com/es/2021.1/Manual/com.unity.visualscripting.html
10. Unity, Burst 1.8 manual — https://docs.unity3d.com/Packages/com.unity.burst@1.8/manual/index.html
11. Godot, "Godot 4.0 will discontinue VisualScript", 23 Aug 2022 — https://godotengine.org/article/godot-4-will-discontinue-visual-scripting/
12. Godot 4.7 docs, "Scripting languages" — https://docs.godotengine.org/en/stable/getting_started/step_by_step/scripting_languages.html
13. Luau: README — https://github.com/luau-lang/luau — Performance — https://luau.org/performance — Sandboxing — https://luau.org/sandbox — News — https://luau.org/news — "Luau Recap for 2025: Runtime", 19 Dec 2025 — https://luau.org/news/2025-12-19-luau-recap-runtime-2025
14. Luau VM sources: `lstate.cpp`, `lstring.cpp`, `ltable.cpp`, `lmathlib.cpp` — https://github.com/luau-lang/luau/tree/master/VM/src
15. Roblox DevForum: "General Release: Luau's New Type Solver" (Nov 2025, via search) — https://devforum.roblox.com/t/general-release-luaus-new-type-solver/4084991 — "Luau Native Code Generation Preview [Studio Beta]" — https://devforum.roblox.com/t/luau-native-code-generation-preview-studio-beta/2572587
16. Roblox, "Parallel Luau" — https://create.roblox.com/docs/scripting/multithreading
17. Remedy, "How Northlight makes Alan Wake 2 shine", 6 Nov 2023 — https://www.remedygames.com/article/how-northlight-makes-alan-wake-2-shine
18. O3DE, Scripting — https://docs.o3de.org/docs/user-guide/scripting/
19. Crytek, "Looking ahead: The Schematyc Beta release in CE 5.3" (search) — https://www.cryengine.com/news/looking-ahead-the-schematyc-beta-release-in-ce-53
20. Defold, "Lua in Defold" — https://defold.com/manuals/lua/
21. Nexus Mods, "Starfield Creation Kit is now available", June 2024 (search snippets) — https://www.nexusmods.com/news/14993
22. Bohemia Interactive, "Understanding Moddability in Enfusion-Powered Games", 7 Jan 2025 — https://reforger.armaplatform.com/news/modding-boot-camps-introduction — "Enforce Script Syntax" (search) — https://community.bistudio.com/wiki/Enscript
23. CK3 Wiki, "Scripting" — https://ck3.paradoxwikis.com/Scripting
24. Norbyte, BG3 Script Extender — https://github.com/Norbyte/bg3se — BG3 Community Wiki, "Understanding Osiris Rules" (search) — https://wiki.bg3.community/Tutorials/Osiris/Understanding-Osiris-Rules
25. Factorio Lua API: "Storage" — https://lua-api.factorio.com/latest/auxiliary/storage.html — `LuaBootstrap` — https://lua-api.factorio.com/latest/classes/LuaBootstrap.html
26. Teardown, Modding — https://teardowngame.com/modding/
27. Noita Wiki, "Modding: Lua API" (search) — https://noita.wiki.gg/wiki/Modding:_Lua_API
28. Kahlua2 — https://github.com/krka/kahlua2
29. Warcraft Wiki, "Secure Execution and Tainting" — https://warcraft.wiki.gg/wiki/Secure_Execution_and_Tainting — "Lua" (search) — https://warcraft.wiki.gg/wiki/Lua
30. Facepunch, s&box "Code" — https://sbox.game/dev/doc/code/
31. Space Station 14, "Sandboxing" — https://docs.spacestation14.com/en/robust-toolbox/sandboxing.html
32. Microsoft Learn, "Code Access Security policy compatibility and migration" (search) — https://learn.microsoft.com/dotnet/framework/misc/code-access-security-policy-compatibility-and-migration
33. Microsoft Learn, Minecraft "Script API Reference Documentation" — https://learn.microsoft.com/minecraft/creator/scriptapi — Bedrock Wiki, "Engine Environment" (search) — https://wiki.bedrock.dev/scripting/api-environment
34. Hypixel Studios (Slikey), "Hytale Modding Strategy and Status", 20 Nov 2025 — https://hytale.com/news/2025/11/hytale-modding-strategy-and-status
35. SpacetimeDB FAQ (search) — https://spacetimedb.com/docs/intro/faq — Clockwork Labs, BitCraftPublic — https://github.com/clockworklabs/BitCraftPublic
36. AmbientRun, Ambient (search) — https://github.com/AmbientRun/Ambient
37. `bevy_mod_scripting` — https://github.com/makspll/bevy_mod_scripting
38. Flecs, "Flecs Script" — https://www.flecs.dev/flecs/FlecsScript.html
39. Lua 5.5 readme — https://www.lua.org/manual/5.5/readme.html — manual — https://www.lua.org/manual/5.5/manual.html — versions — https://www.lua.org/versions.html — Lua 5.4 `lstate.c` — https://github.com/lua/lua/blob/v5.4/lstate.c
40. LuaJIT, "Status" — https://luajit.org/status.html — "Installation" — https://luajit.org/install.html
41. `mlua` — https://github.com/mlua-rs/mlua — https://docs.rs/mlua/latest/mlua/struct.Lua.html — `rlua` — https://github.com/mlua-rs/rlua
42. piccolo — https://github.com/kyren/piccolo
43. Wasmtime: release process — https://docs.wasmtime.dev/stability-release.html — `Config` — https://docs.wasmtime.dev/api/wasmtime/struct.Config.html — Pulley — https://docs.wasmtime.dev/examples-pulley.html — debugging — https://docs.wasmtime.dev/examples-debugging-native-debugger.html — licence — https://github.com/bytecodealliance/wasmtime/blob/main/Cargo.toml
44. WebAssembly Core Specification, "Instructions" — https://webassembly.github.io/spec/core/syntax/instructions.html — W3C WebAssembly WG, IPR — https://www.w3.org/groups/wg/wasm/ipr
45. `wasmer` — https://docs.rs/crate/wasmer/latest — `wasmi` — https://github.com/wasmi-labs/wasmi — Extism — https://github.com/extism/extism
46. Rhai: benchmarks — https://rhai.rs/book/about/benchmarks.html — features — https://rhai.rs/book/start/features.html — object maps — https://rhai.rs/book/language/object-maps.html
47. Rune — https://github.com/rune-rs/rune — `runtime::budget` — https://docs.rs/rune/latest/rune/runtime/budget/index.html
48. Mun — https://github.com/mun-lang/mun — `mun_runtime` — https://docs.rs/crate/mun_runtime/latest — Koto — https://koto.dev — Dyon — https://github.com/PistonDevelopers/dyon — Gluon — https://github.com/gluon-lang/gluon
49. `starlark-rust` — https://github.com/facebook/starlark-rust — `rquickjs` — https://docs.rs/crate/rquickjs/latest — `boa_engine` — https://docs.rs/crate/boa_engine/latest — `netcorehost` — https://docs.rs/crate/netcorehost/latest
50. LuaLS — https://github.com/LuaLS/lua-language-server — Teal — https://github.com/teal-language/tl — `luau-lsp` — https://github.com/JohnnyMorganz/luau-lsp
51. ink — https://github.com/inkle/ink — `blade-ink-rs` — https://github.com/bladecoder/blade-ink-rs — `YarnSpinner-Rust` — https://github.com/YarnSpinnerTool/YarnSpinner-Rust
52. Epic Games, "StateTree" (UE 5.8) — https://dev.epicgames.com/documentation/en-us/unreal-engine/state-tree-in-unreal-engine
53. Patents:
    - US 10,909,754 B1 — https://patents.google.com/patent/US10909754B1/en
    - US 11,144,284 B1 — https://patents.google.com/patent/US11144284B1/en
    - US 11,755,313 B2 — https://patents.google.com/patent/US11755313
    - US 11,847,433 B2 — https://patents.google.com/patent/US11847433
    - EP 4371005 — https://data.epo.org/publication-server/rest/v1.2/patents/EP4371005NWA1/document.html
    - US 8,449,397 — https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/8449397
    - Zynga's grants on Justia — https://companyprofiles.justia.com/company/zynga/patents/patent?page=25
    - US 11,579,866 B2 — https://patents.google.com/patent/US11579866B2/en
    - US 11,027,196 B2 — https://patents.google.com/patent/US11027196
    - US 9,459,935 B2 — https://patents.google.com/patent/US9459935
    - US 9,639,432 B2 — https://patents.google.com/patent/US9639432
    - US 8,819,641 B1 — https://patents.google.com/patent/US8819641B1/en
    - Portfolio queries — https://patents.google.com/xhr/query
54. "Lua in World of Warcraft", Lua Workshop 2008 (search) — https://www.lua.org/wshop08/lua-whitehead.pdf
55. Tom's Hardware, "CD Projekt RED Confirms Next 'The Witcher' Game Will Use Unreal Engine 5", March 2022 (search) — https://tomshardware.com/news/cd-projekt-red-next-witcher-unreal-engine
