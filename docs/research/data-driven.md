# Data-driven engine, game code and mods

An annotated bibliography for the owner's question on issue #64 — *make Forge adaptable to various
projects, data-driven, moddable: how do recent engines work?* It covers three things: how engines
separate **engine, game and content** (type systems, reflection, schemas and serialisation, asset ids
and references, hot reload, editors); how **game code** is attached to an engine (native plugins,
embedded scripting, WebAssembly, hot reload of Rust) and what each costs in determinism and
networking; and how **shipped games open themselves to modders** (load orders and conflicts,
versioning, sandboxing, distribution). Companion to [procedural.md](procedural.md) §6, which already
cites Our Machinery's "The Truth", "Creation Graphs" and "DLL Hot Reloading" and Frykholm's
data-oriented entity series — those are referenced here and not repeated — and to
[memory-streaming.md](memory-streaming.md) §4–5 (Decima's content graph, the cooked container),
[netcode.md](netcode.md) §5 (determinism) and [audio.md](audio.md) (the RON event data layer). Labels
follow the house convention — **[paper] [book] [talk] [web] [code] [docs]** and **foundational /
still-current / recent** — and every citation was confirmed against at least one reachable page;
what could not be confirmed is listed at the end, with what was tried.

> **State of the art in five sentences.** Every engine that serves more than one game puts a
> reflected type system between code and content — Unreal's UHT-generated reflection, Unity's
> serialised classes and ScriptableObjects, Godot's Resources, O3DE's SerializeContext, Bevy's
> `bevy_reflect` — so that one declaration feeds loading, saving, the editor, networking and
> scripting, and all of them now reference content by stable id rather than by path (Unity's asset
> IDs in `.meta` files, Unreal's Primary Asset Ids, Godot's `uid://`, extended to scripts and
> shaders only in 4.4, 2025). Authoring data
> and runtime data are different shapes — Frostbite's "three data schemas", Unity's baking, Dungeon
> Siege's templates compiled into a content database in 2002 — so the pattern is a tolerant,
> versioned, human-editable format compiled into flat tables, with templates and sparse overrides
> (USD's layers and `over`, Our Machinery's prototypes, Bevy 0.19's BSN patches) as the composition
> primitive. For game code the choice is native plugins (fast and unsafe, and in Rust, which has no
> stable ABI, a development tool: `subsecond` hot-patches Bevy systems since 0.17), an embedded
> interpreter (Lua and Luau: small and sandboxable, but Factorio had to replace Lua's maths and
> iteration order to keep lockstep), or WebAssembly (wasmtime: a sandbox, fuel metering and, since
> Wasm 3.0 in September 2025, a standard deterministic profile; shipped in Microsoft Flight
> Simulator's add-ons and Veloren's plugins). Shipped games open themselves through *data layers
> with a load order and a conflict rule* — Bethesda's "rule of one", Paradox's LIOS/FIOS, Minecraft
> data packs where the last pack wins except tags, which merge, RimWorld's XPath patches, Factorio's
> three data rounds sorted by dependency depth — and a lockstep game like Factorio refuses to start
> unless every peer runs the same mods with matching checksums. Where strangers' code runs, safety
> comes from a sandbox and server authority (Quake III's QVM, Luau, Roblox, Verse, Veloren), never
> from trust — the 2023 *fractureiser* malware in Minecraft mods is what native-code mods on an open
> portal cost — and long-lived ecosystems enforce compatibility in the toolchain (Verse rejects
> breaking changes at publication; The Machinery versions every API by semver).

**Contents**

1. [Engine, game and content in the big engines](#1-engine-game-and-content-in-the-big-engines)
2. [Data models, schemas and serialisation](#2-data-models-schemas-and-serialisation)
3. [Game code: native plugins, scripting, WebAssembly, hot reload](#3-game-code-native-plugins-scripting-webassembly-hot-reload)
4. [How shipped games open themselves to mods](#4-how-shipped-games-open-themselves-to-mods)
5. [Distribution and safety](#5-distribution-and-safety)
6. [Recommendation for Forge](#recommendation-for-forge)
7. [Checked and left out](#checked-and-left-out)
8. [Verification notes](#verification-notes)

---

## 1. Engine, game and content in the big engines

The owner's question has two halves. "Adaptable to various projects" is about the line between the
engine (mechanisms), a game (rules and systems) and content (the rows those systems read). Every
general-purpose engine draws it with the same three tools: a reflected type system, stable asset ids,
and a pipeline that turns editable data into runtime data. "Mod-able" is the same line opened to
people outside the team, and is covered in §3–5. The oldest entries here are still the clearest
statements of the idea.

**Scott Bilas. "A Data-Driven Game Object System." *GDC 2002*, Gas Powered Games.** [talk]
[foundational]
<https://www.gamedevs.org/uploads/data-driven-game-object-system.pdf>

Dungeon Siege's object system: over 7,300 object types placeable in the editor and over 100,000
placed objects in a continuous world. Bilas defines data-driven as "no engineer required", argues
that every class hierarchy for game objects eventually turns out wrong, and concludes that the object
system is a database whose *schema*, not only its values, must be data. Components are assembled by
data; templates live in a specialisation tree of text files, each overriding its base's fields; every
component field has a schema entry (type, default, flags, doc string); placed instances store only
what differs from their template, so editing a template changes every instance; the editor's
property sheet is generated from the schema, and scripted components look the same as C++ ones to
the editor. The pitfalls slide is the honest part: the system was designed for under a hundred
templates and shipped with thousands, many of them generated.
*Bearing:* the whole recipe in 2002 form — a typed schema with docs, template inheritance with sparse
overrides, instances saved as deltas, the editor generated from the schema. Forge's material table
(D-007) is already the first such table; "designed for 100, got 7,300" argues for tooling that shows
templates and their overrides.

**Dan Liebgold. "Adventures in Data Compilation and Scripting for UNCHARTED: DRAKE'S FORTUNE."
*GDC 2008*, Naughty Dog.** [talk] [foundational]
<https://archive.org/details/GDC2008Liebgold>

DC, Naughty Dog's data compiler: game data written in a Scheme-based specification language, checked
by the compiler the way code is, and compiled to binary for the PS3 runtime; a scripting system built
into the same compiler, exploiting the duality of code and data; used for animation states, blend
trees, particle definitions and gameplay scripts, with live data updates while tuning.
*Bearing:* proof that a *type-checked data language compiled to flat binary* carries a AAA
production. Forge needs no new language for it: Rust structs with `serde` derives are the schema, RON
is the text, and the loader compiles it into tables; the checking DC did is Rust's type system plus
reference validation at load.

**Michael Noland. "Unreal Property System (Reflection)." Unreal Engine blog, 27 March 2014.** [web]
[foundational]
<https://www.unrealengine.com/en-US/blog/unreal-property-system-reflection> — author's copy:
<http://michaelnoland.com/unreal-property-system-reflection/>

Unreal's reflection is opt-in: `UCLASS`, `USTRUCT`, `UPROPERTY`, `UFUNCTION` and `UENUM` markers, a
`.generated.h` include, and the Unreal Header Tool, which parses headers and emits reflection data
compiled into the same binary, so it cannot drift from the code. That one declaration feeds the
editor's details panels, serialisation, garbage collection, network replication and Blueprint/C++
communication. UHT understands only a subset of C++ (a few templates such as `TArray`).
*Bearing:* the canonical case for reflection as the hub of an engine. Forge gets the same "declare
once" property from derive macros — `serde` today, `bevy_reflect` when tools need it — and should not
build a header-parsing code generator of its own.

**Epic Games. "Asset Management in Unreal Engine." Unreal Engine 5.8 documentation.** [docs]
[still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/asset-management-in-unreal-engine>

Primary assets are addressed by a **Primary Asset ID** — a type naming a group of assets plus the
asset's name — and are discovered, loaded and unloaded explicitly through the Asset Manager; secondary
assets load only because a primary one references them. `UPrimaryDataAsset` is the base class for
data-only primary assets and carries asset bundles, which also drive cooking and chunking. By default
only `UWorld` assets are primary.
*Bearing:* Forge's id should be the same pair — a table (the type) and a name — and the loader should
distinguish what is loaded on purpose (a world, a package) from what is loaded because it is
referenced.

**Epic Games. "Game Features and Modular Gameplay in Unreal Engine"; "Game Framework Component
Manager in Unreal Engine." Unreal Engine 5.8 documentation.** [docs] [recent]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/game-features-and-modular-gameplay-in-unreal-engine>
— <https://dev.epicgames.com/documentation/en-us/unreal-engine/game-framework-component-manager-in-unreal-engine>

A Game Feature plugin is a self-contained feature whose `GameFeatureData` asset lists actions — add
components to actor classes, add data registries or registry sources, add cheats, add World Partition
content — applied when the feature is activated and undone when it is deactivated. Underneath, actors
register themselves as *receivers* with the Game Framework Component Manager, and a feature's
extension handlers add components to them, so the actor class never names the feature.
*Bearing:* the dependency direction Forge wants between engine, game and optional content: the base
exposes extension points and features attach to them, never the reverse. In ECS terms, a package adds
components, systems and table rows to things it does not own.

**Unity Technologies. "ScriptableObject"; "Asset metadata" (Unity 6.2 manual); Addressables 2.7
manual.** [docs] [still-current]
<https://docs.unity3d.com/6000.2/Documentation/Manual/class-ScriptableObject.html> —
<https://docs.unity3d.com/6000.2/Documentation/Manual/AssetMetadata.html> —
<https://docs.unity3d.com/Packages/com.unity.addressables@2.7/manual/index.html>

ScriptableObject is Unity's data container independent of scene objects: saved as an asset, shared
by reference instead of copied per instance, edited in the Inspector, and read-only in a built player,
where changes made at run time are not saved. Every asset has a `.meta` sidecar holding its unique ID
and import settings; references go through that ID, so assets can be moved in the editor, but an
asset moved outside the editor without its `.meta`, or a lost `.meta`, breaks every reference to it.
Addressables add a location-independent *address* per asset on top of AssetBundles, loadable locally
or from a CDN.
*Bearing:* ids must survive moves, and sidecar files are the weak point of making them do so. Logical,
namespaced names (`package:thing`) give move-proof references without sidecars.

**Unity Technologies. "Baking overview." Entities 1.3 manual.** [docs] [recent]
<https://docs.unity3d.com/Packages/com.unity.entities@1.3/manual/baking-overview.html>

DOTS separates *authoring* data (editable GameObjects) from *runtime* data (entities and components):
bakers convert one into the other, in the editor only, so a running game never sees authoring
objects. Open subscenes bake live and incrementally, closed ones in the background. The manual warns
that incremental and full baking can produce different entity orders and chunk layouts, so baking code
must not rely on order.
*Bearing:* the authoring/runtime split in shipped form. The ordering caveat is a determinism warning
for Forge: the step that merges packages into runtime tables must give the same indices whatever order
the files were read in (see the Recommendation).

**Godot Engine. "Resources" (stable documentation); Hugo Locurcio, "UID changes coming to Godot
4.4", 15 January 2025.** [docs] [web] [recent]
<https://docs.godotengine.org/en/stable/tutorials/scripting/resources.html> —
<https://godotengine.org/article/uid-changes-coming-to-godot-4-4/>

A Resource is a data container — serialisable, reference-counted, loaded once and shared — whose
exported properties are edited in the Inspector; custom Resource scripts are the docs' own analogue of
Unity's ScriptableObjects, saved as text `.tres` or binary `.res`. A resource file stores the path of
the script that defines its type and loads that script along with the file. Godot referred to
files by path for most of its life: path references broke when files moved outside the editor, and
the editor's automatic fix-up was slow and unreliable in large projects. Partial UID support arrived
in 4.0 for scenes and most resources; scripts and shaders, being plain text with nowhere to store an
id, got `.uid` sidecar files only in 4.4, together with `uid://` references in code.
*Bearing:* Godot added ids late and in two steps, with a migration for existing projects each time;
decide ids before content exists. And a data file that names a script is a code path: Forge's data
files should stay inert (§5).

**Godot Engine. "What is GDExtension?" Godot 4.4 documentation.** [docs] [still-current]
<https://docs.godotengine.org/en/4.4/tutorials/scripting/gdextension/what_is_gdextension.html>

GDExtension loads native shared libraries at run time through a C interface
(`gdextension_interface.h`, `extension_api.json` and a `.gdextension` file), so extensions need no
engine rebuild; `godot-cpp` is the official binding and others are community-made. Compatibility is
forward across minor versions (a 4.2 extension should load in 4.3) and never backward, and even the
forward promise has been broken once: 4.0 extensions do not load in 4.1.
*Bearing:* a C ABI makes binary plugins possible but turns compatibility into a promise the engine
team must keep at every release. Forge, in Rust without a stable ABI (§3), should not make that
promise.

**Bevy contributors. `bevy_reflect` 0.19.1 (13 August 2026).** [code] [recent]
<https://docs.rs/bevy_reflect/latest/bevy_reflect/>

`#[derive(Reflect)]` gives structs and enums run-time introspection (`Reflect`, `PartialReflect`);
dynamic types (`DynamicStruct` and the rest) stand in for values whose type is known only at run time;
a `TypeRegistry` stores per-type data (`ReflectDefault`, `ReflectSerialize` …) and drives
reflection-based (de)serialisation; function reflection is an optional feature. The crate describes
itself as general-purpose rather than Bevy-only; reflected types must be `'static`. Since Bevy 0.17
every type deriving `Reflect` is registered automatically, generic types excepted.
*Bearing:* the reflection layer Forge already has one crate away — the same release as the
`bevy_ecs` 0.19 of D-006 — for an inspector, scene files or a scripting bridge. Loading tables does
not need it; `serde` does that.

**Bevy contributors. `bevy_asset` 0.19.1.** [code] [recent]
<https://docs.rs/bevy_asset/latest/bevy_asset/>

Strong handles are reference-counted and an asset is dropped with the last one (the docs name both
failure modes: never dropping, dropping too early); `AssetId` is the cheap copyable id;
`AssetServer::load` deduplicates; loaders are separate types with their own settings; an asset
processor turns authored files into game-ready ones, configured by `.meta` files; changed files are
hot-reloaded behind the `file_watcher` feature; assets declare dependencies and expose labelled
sub-assets.
*Bearing:* a complete, readable Rust reference for the asset side. Forge's streaming (D-018, D-025)
works on cells and pages rather than handles, but the processor/meta split and labelled sub-assets
port directly.

**Bevy contributors. "Bevy 0.19" (19 June 2026); `bevy_app` `Plugin` trait.** [web] [code] [recent]
<https://bevy.org/news/bevy-0-19/> — <https://docs.rs/bevy_app/latest/bevy_app/trait.Plugin.html> —
first release (0.1, 10 August 2020): <https://bevy.org/news/introducing-bevy/>

0.19's headline is BSN, the new scene notation, written through a `bsn!` macro: scenes are
*composable patches* that write fields over defaults and over other scenes, components are built
from templates with access to the world (an asset path becomes a handle), and entities can be named
and referenced inside a scene. The post is explicit that a first-party `.bsn` asset loader has not
shipped — the release is code-first — and that BSN is the base of the coming Bevy editor. A `Plugin`
configures an `App` in `build`, with `ready`, `finish` and `cleanup` for staged start-up.
*Bearing:* two signals. Patching is where scene composition has landed (as in USD and The Machinery,
§2); and six years after its first release, Bevy still has no shipped editor — the editor, not the
data model, is the long pole. Forge can have the data model now and the editor later.

**Open 3D Engine. "Gems"; "Gem Versioning and Compatibility"; "Serialization Context".** [docs]
[still-current]
<https://www.docs.o3de.org/docs/user-guide/gems/> —
<https://www.docs.o3de.org/docs/user-guide/gems/gem-versioning/> —
<https://www.docs.o3de.org/docs/user-guide/programming/components/reflection/serialization-context/>

O3DE is assembled from Gems: packages of code and/or assets with a `gem.json` manifest, semantic
versions, and `compatible_engines` and `engine_api_dependencies` lists with PEP 440 version
specifiers; the highest compatible version wins, and the tools refuse to register an incompatible gem
unless forced. Components reflect themselves into three contexts: `SerializeContext` (persistence,
fields registered by name), an edit context (editor UI) and `BehaviorContext` (Lua and Script
Canvas).
*Bearing:* the package manifest Forge's content packages need. O3DE separates the *engine version*
from the *engine API version* a package depends on — the right split, since most engine releases do
not change the data schema.

**Ludovic Chabant. "Tools Tutorial Day: A Tale of Three Data Schemas." *GDC 2018*, Electronic Arts;
Matthew Doell. "Frostbite: Implementing a Scripting Solution for Your Editor." *GDC 2015*,
Electronic Arts.** [talk] [still-current]
<https://www.gdcvault.com/play/1025284/Tools-Tutorial-Day-A-Tale> —
<https://www.gdcvault.com/play/1021996/Frostbite-Implementing-a-Scripting-Solution>

What Frostbite has made public about its data side. Chabant's thesis: every logical piece of data
has three schemas — the one the runtime consumes, the one on disk, and the one content creators see —
and getting each right, with examples from EA games on Frostbite, solved production problems. Doell
presents FBScript, the scripting layer of Frostbite's tools, used to build per-project tools and share
them between Battlefield 4, Need for Speed, Plants vs. Zombies: Garden Warfare and Dragon Age:
Inquisition. Only the Vault abstracts are verified.
*Bearing:* name the three schemas in Forge from the start: the RON file (storage, the only persistent
one, so the one to version), the runtime table (free to change at every release) and the editor's view
(later). A GPU row layout never appears in a file.

**Dan Sumaili, Sander van der Steen. "Creating a Tools Pipeline for Horizon Zero Dawn." *GDC 2017*,
Guerrilla Games.** [talk] [still-current]
<https://www.guerrilla-games.com/read/creating-a-tools-pipeline-for-horizon-zero-dawn>

Guerrilla rebuilt Decima's tools pipeline from scratch when it moved from linear shooters to an open
world RPG, as a framework on which an integrated development environment was built. The slides
(linked from the page) were too large to read; what is public of Decima's data at scale — 300,000
files, 16 million objects, 20 million links in acyclic content graphs — is in
[memory-streaming.md](memory-streaming.md) §4.
*Bearing:* the precedent that an open world forced a rebuild of the tools and data framework, not
only of the renderer. Forge is choosing its data framework before its first open world, which is the
cheap moment.

---

## 2. Data models, schemas and serialisation

[procedural.md](procedural.md) §6 already holds the central design documents: Gray's "The Story
behind The Truth" (typed objects, UUID references, every change an `(object, property, old, new)`
record, hence undo and collaboration for free), Persson's "Creation Graphs" and Gray's "DLL Hot
Reloading in Theory and Practice", plus Frykholm's Bitsquid series, whose Part 4 compiles entity
definitions authored as data. This section adds the later Machinery posts on plugins, versioning and
prototypes, the one Rust engine with a shipped editor, and the formats and rules for evolving data.

**Niklas Gray. "Little Machines Working Together (Part 1)." Our Machinery blog, 16 May 2017.** [web]
[still-current as design; **site dead, cite the archive**]
<https://ruby0x1.github.io/machinery_blog_archive/post/little-machines-working-together-part-1/index.html>

The Machinery's plugin architecture: every subsystem is a plugin in its own DLL exposing one API — a C
struct of function pointers — registered by name in a central API registry; plugins never depend on
each other directly, only on APIs fetched from the registry; an interface can have several
implementations (the real one, a test double); and the indirection is what turns hot reloading into a
pointer swap.
*Bearing:* the shape of Forge's game-to-engine seam even with static linking: engine services and game
systems registered behind traits, so a game, a demo or a test can replace one. Bevy's `Plugin` is the
Rust idiom for the same thing.

**Niklas Gray. "API Versioning." Our Machinery blog, 22 September 2021.** [web] [still-current]
<https://ruby0x1.github.io/machinery_blog_archive/post/api-versioning/index.html>

Every API in the registry carries a semantic version and a plugin asks for the version it was built
against: the major must match exactly; a higher minor satisfies a lower request (minors only add
functions); a request for a newer minor than exists fails; 0.x APIs must match exactly. A plugin whose
required APIs are missing is disabled together with its exports, the failure reported down the
dependency chain, and optional requests let one plugin support several versions.
*Bearing:* the rule set for Forge's package manifests and, later, a mod API: semver per interface
rather than per engine, exact major, minimum minor, and a report instead of a crash.

**Niklas Gray. "Prototypes in The Machinery." Our Machinery blog, 29 June 2020.** [web]
[still-current]
<https://ruby0x1.github.io/machinery_blog_archive/post/prototypes-in-the-machinery/index.html>

Any object in The Truth can have a prototype: it stores a reference to it plus a bit mask of
overridden properties, inherits everything else through the whole prototype chain, and sees edits to
the prototype immediately unless it overrode them. Sets are overridden by adding and removing
elements rather than by replacement; sub-objects can be inherited, removed or instantiated locally.
It applies to all data, not only entities, and the property editor greys inherited values and offers
reset and propagate.
*Bearing:* the exact semantics a mod "patch" needs — a sparse override of named fields over a base
record, sets edited by add/remove, edits to the base flowing through — and the same mechanism serves
prefabs and material variants.

**Fyrox contributors. *The Fyrox Book*: "Plugins"; "Hot Reloading".** [docs] [still-current]
<https://fyrox-book.github.io/scripting/plugin.html> —
<https://fyrox-book.github.io/beginning/hot_reloading.html>

Fyrox is a Rust engine with a shipped editor. A game is a plugin deriving `Reflect` (compile-time
reflection that lets the editor inspect and set fields) and `Visit` (Fyrox's serialisation, used for
scenes and for carrying state across reloads). Code hot reload builds the game as a dynamic library:
before unloading, the plugin's state is serialised with `Visit`; the new library is loaded and the
state restored. The book calls it experimental and memory-unsafe, names trait objects (their vtables
point into the old library) and statics as the usual breakers, and recommends static linking when it
misbehaves.
*Bearing:* the most complete Rust precedent for a reflection-driven editor with code hot reload — and
its own warnings are why Forge's hot reload should be data first and code second.

**Alliance for OpenUSD / Pixar. "Glossary." OpenUSD documentation.** [docs] [still-current]
<https://openusd.org/release/glossary.html>

USD composes a scene from layers. A LayerStack is an ordered set of layers holding sparse
*opinions*; an attribute's value is the strongest opinion; an `over` adds opinions to a prim without
defining it; composition arcs have a fixed strength order — LIVRPS: local, inherits, variant sets,
references, payloads, specializes; variant sets switch between alternatives; payloads defer loading;
sublayers can prepend, append, remove or reset list entries sparsely.
*Bearing:* the most rigorous published model of many people editing one world without touching each
other's files — which is what mods are. Forge needs only the local-layer subset: an ordered stack of
packages, sparse overrides, list edits, with the load order as the strength order.

**Blender. "DNA." Blender Developer Documentation.** [docs] [foundational]
<https://developer.blender.org/docs/features/core/dna/>

At build time `makesdna` parses Blender's C struct headers into SDNA, a compact description of every
persistent struct, and every `.blend` embeds the SDNA it was written with. On load the file's SDNA is
compared with the running binary's and data is converted field by field; renames are listed in
`dna_rename_defs.h`, semantic upgrades live in versioning code (`versioning_*.cc`, `do_versions`).
The cost: DNA mirrors raw C layout, so padding, pointer size and endianness matter.
*Bearing:* the strongest example of a file format that carries its own schema, and the reason old
`.blend` files still open in new Blenders. Forge's text records get the same with less machinery:
field names are in the file, renames go in an alias list, semantic changes go in per-table
migrations keyed by schema version.

**Google. "Language Guide (proto 3)", section "Updating A Message Type." protobuf.dev.** [docs]
[still-current]
<https://protobuf.dev/programming-guides/proto3/>

The standard rules for evolving a schema without breaking old data: never change an existing field's
number, since it identifies the field on the wire; reserve the numbers and names of deleted fields so
they are never reused; adding fields and enum values is safe; some type changes are wire-compatible
if deployment is managed.
*Bearing:* Forge's records are keyed by name rather than number, so the rules become: never rename
without an alias, never reuse a removed name with a new meaning, add fields only with a default, never
change a field's type within a schema version.

**serde contributors. "Serde data model"; RON contributors, `ron` 0.12; `rkyv` 0.8.18.** [code]
[still-current]
<https://serde.rs/data-model.html> — <https://github.com/ron-rs/ron> —
<https://docs.rs/rkyv/latest/rkyv/>

`serde` maps Rust types into a 29-type data model and formats map that model to bytes; schemas and
versions are left to the application. RON is a Rust-shaped text format — structs, enums, tuples,
maps, comments, trailing commas, unquoted field names — compatible with serde, with documented gaps
(internally tagged and untagged enums, `#[serde(flatten)]`). `rkyv` is zero-copy deserialisation
with optional validation (`bytecheck`); archived data stays readable only while the schema is
unchanged.
*Bearing:* RON with serde is the authoring and storage format (audio.md already chose it, and serde is
a workspace dependency); rkyv is for cooked runtime tables only if loading them ever shows in a
profile, never for anything a person or a mod edits.

**fasterthanlime and contributors. `facet` 0.46.5 (31 July 2026).** [code] [recent]
<https://docs.rs/facet/latest/facet/>

`#[derive(Facet)]` gives a type an associated `SHAPE` constant describing its layout, fields, doc
comments and attributes, from which serialisers for many formats, pretty-printing, diffing and CLI
parsing are built, including constructing values of arbitrary shape in safe code. The core crate is
fully documented and stable; many satellite crates are marked experimental.
*Bearing:* a compile-time reflection alternative to `bevy_reflect` that keeps doc comments — useful
for an editor that shows field documentation, as Bilas' schema did. One to re-evaluate when the editor
starts; not a dependency now.

---

## 3. Game code: native plugins, scripting, WebAssembly, hot reload

Content is data; rules are partly data (P6) and partly code. The code can be linked natively, run in
an embedded interpreter, or run in a sandboxed virtual machine. The sources below give the
trade-offs; the table at the end puts them side by side for Forge, whose simulation must be
deterministic (P3) and shared by client and server (P8).

**Robert Nystrom. "Bytecode." *Game Programming Patterns*, 2009–2014 (free web edition).** [book]
[foundational]
<https://gameprogrammingpatterns.com/bytecode.html>

The case for putting behaviour in a virtual machine: iteration without recompiling, a sandbox so
user-made behaviour cannot crash or subvert the game, changes after release, and mods — against real
costs: slower than native code, a compiler and tools to build, and poor debugging without them. The
chapter's warning is the one to remember: keep the language small, or it grows into a badly designed
general-purpose one.
*Bearing:* the decision rule for Forge: script only what must change without a rebuild *and* must be
safe; everything else stays Rust.

**Fabien Sanglard. "Quake 3 Source Code Review: Virtual Machine." fabiensanglard.net, 30 June
2012.** [web] [foundational]
<https://fabiensanglard.net/quake3/qvm.php>

id Tech 3 ran its game (server logic), cgame (client gameplay) and UI modules in a virtual machine:
C compiled by lcc into a stack bytecode, interpreted or compiled to x86 at load time, with native DLLs
still possible; the modules reach the engine only through system calls. The design combined the
safety and portability of Quake's VM with the speed of Quake II's native DLLs; mods shipped as
bytecode ran on every platform without access to memory outside their sandbox.
*Bearing:* WebAssembly under wasmtime is the same design with a standard behind it — a systems
language compiled to portable bytecode, compiled to native code at load, reaching the engine only
through an import table — and id Tech 3 is the evidence that it suits game code.

**Roberto Ierusalimschy, Luiz Henrique de Figueiredo, Waldemar Celes. "A Look at the Design of
Lua." *Communications of the ACM* 61(11), 2018.** [paper] [still-current]
<https://doi.org/10.1145/3186277> — listed at <https://www.lua.org/docs.html>

Lua is implemented as a library with a C API, not as a program — about 25,000 lines of C; tables are
its single data structure; asymmetric coroutines let a script suspend from inside nested calls, which
games use to run each character's script in its own coroutine resumed every update. The authors call
it the leading scripting language in games.
*Bearing:* why Lua is the default embedded language: small, embeddable, with coroutines that fit game
scripts. What it does not give Forge is determinism across machines — Factorio replaced its maths
functions and its table iteration order to get it (§4).

**Luau contributors (Roblox). "Sandboxing" (luau.org); "Luau Goes Open-Source", 3 November 2021;
`mlua` 0.12.1 (29 August 2026).** [docs] [code] [recent]
<https://luau.org/sandbox> — <https://luau.org/news/2021-11-03-luau-goes-open-source/> —
<https://docs.rs/mlua/latest/mlua/>

Luau is Roblox's evolution of Lua 5.1 with gradual types, MIT-licensed since November 2021. Its
sandbox: `io` and `package` removed, file loading removed, `os` and `debug` cut to a few functions;
the standard libraries and the global table read-only, each script with its own globals; no loading of
bytecode, because untrusted bytecode is hard to validate; `__gc` removed; an interrupt the host can
install to stop runaway scripts; thread identities to guard privileged APIs. `mlua` binds Lua 5.1–5.5,
LuaJIT and Luau to Rust, with serde conversion and async functions.
*Bearing:* if Forge ever embeds a designer language, Luau through `mlua` is the reference for doing it
safely, and this sandbox list is the checklist whatever language is chosen.

**Rhai contributors. Rhai 1.26.1 (10 September 2026); *The Rhai Book*, "Features".** [code] [recent]
<https://docs.rs/rhai/latest/rhai/> — <https://rhai.rs/book/about/features.html>

A Rust-native embedded language: an engine declared immutable cannot mutate its host unless allowed;
limits against runaway scripts, deep recursion and oversized data; direct registration of Rust types,
methods, getters and indexers; language features (floating point, arrays and others) can be compiled
out; a "don't panic" guarantee towards the host. It is an AST interpreter with an experimental
bytecode mode.
*Bearing:* the lowest-friction option for small, trusted, host-side scripts in pure Rust — console
commands, debug tools, test scenarios. Not a mod platform.

**Bytecode Alliance. "Deterministic Wasm Execution" (Wasmtime documentation); `wasmtime` 49.0.1
(24 September 2026); Andreas Rossberg, "Wasm 3.0 Completed", webassembly.org, 17 September 2025.**
[docs] [code] [recent]
<https://docs.wasmtime.dev/examples-deterministic-wasm-execution.html> —
<https://docs.rs/wasmtime/latest/wasmtime/> — <https://webassembly.org/news/2025-09-17-wasm-3.0/>

Wasmtime's page lists what is non-deterministic in Wasm — NaN bit patterns, relaxed SIMD, whether
`memory.grow` and `table.grow` succeed below their maximum, and host imports such as clocks and file
systems — and how to remove each: NaN canonicalisation in Cranelift, deterministic relaxed SIMD (or
the proposal disabled), a resource limiter, virtualised imports, and *fuel* rather than epoch
interruption for deterministic time-slicing. The crate runs components as well as modules and has a
`ResourceLimiter` for memories and tables. Wasm 3.0 specifies a deterministic default for every
instruction whose result is otherwise non-deterministic — floating-point NaNs and relaxed vector
instructions — alongside 64-bit memories, garbage collection, tail calls and exceptions.
*Bearing:* the only sandbox here with a *specified* deterministic mode, which is what P3 and P8
require of any mod code that client and server both run. Fuel doubles as a per-tick CPU budget per
mod.

**Bytecode Alliance. *The WebAssembly Component Model* documentation.** [docs] [recent]
<https://component-model.bytecodealliance.org/>

Components are Wasm libraries and applications that interact through typed interfaces written in WIT
and grouped into *worlds*, not through shared memory; guest languages include Rust, C/C++, Go, Python,
JavaScript, C# and MoonBit; the stable WASI release is 0.2.0, of 25 January 2024.
*Bearing:* WIT is the natural way to write a Forge mod API as a set of interfaces, so that a mod
declares exactly which engine capabilities it imports — capability-based sandboxing, with each
interface versioned on its own.

**Microsoft / Asobo. "WebAssembly." Microsoft Flight Simulator SDK documentation.** [docs]
[still-current]
<https://docs.flightsimulator.com/html/Programming_Tools/WASM/WebAssembly.htm>

MSFS moved add-on code from DLLs to WebAssembly for security and for portability, Xbox included:
modules are compiled ahead of time to native code rather than interpreted; they reach the simulator
only through SDK headers (gauges, communication, networking, NanoVG rendering and others); they read
files only inside their own package and write only to a work folder; there is no threading;
`module_init` and `module_deinit` bracket their lifetime.
*Bearing:* a shipped, commercial, console-inclusive precedent for Wasm add-ons at native speed. The
file rule — read your own package, write your own work folder — is one to copy.

**Veloren contributors. "Writing a plugin." *Veloren: An Owner's Manual*.** [docs] [recent]
<https://book.veloren.net/contributors/modders/writing-a-plugin.html>

Veloren, an open-source voxel RPG in Rust, runs plugins compiled to Wasm: sandboxed, and therefore run
client-side automatically; a server sends its plugins to connecting clients; one package can hold
server and client behaviour; the engine talks to plugins through event handlers. The page states the
API is experimental with no stability guarantee, and it still describes the older `wasm32-wasi`
toolchain.
*Bearing:* the Rust-game precedent for Forge's eventual mod-code path, including server-to-client
plugin delivery — and a reminder that an API without stability guarantees keeps mods few.

**Epic Games. *The Book of Verse* (CC0), chapters "Effects" and "Code Evolution and Compatibility".**
[docs] [recent]
<https://verselang.github.io/book/> — <https://github.com/verselang/book>

Verse, the language of UEFN, was designed for a persistent shared world: failure is control flow;
effects (`transacts`, `decides`, `no_rollback`, `reads`, `computes`) are part of a function's type and
let speculative code roll back; concurrency is structured. The evolution chapter is the unusual part:
backward compatibility is enforced at *publication* — public definitions cannot be removed, renamed or
change kind; closed enums stay closed; classes gain fields only with defaults; persistable structs are
frozen once published; a function's effects may only shrink — with deprecations as warnings until a
language-version upgrade, and Epic keeping rare powers to break things for legal or safety reasons.
*Bearing:* the most complete written rule set for a mod API and a save format that must outlive their
authors. Forge's package validator should apply the same rules to public record ids and fields between
minor versions.

**The Rust Reference. "Type layout"; `abi_stable` 0.11.3.** [docs] [code] [still-current]
<https://doc.rust-lang.org/reference/type-layout.html> — <https://docs.rs/abi_stable/latest/abi_stable/>

The default representation guarantees only what soundness requires (alignment, no overlap) and
nothing else about layout; `repr(C)` is the stable layout for interoperation. `abi_stable` builds
Rust-to-Rust dynamic libraries that work across compiler versions on top of that, checking type
layouts when a library loads and extending vtables through "prefix types" — for libraries loaded at
start-up, without unloading.
*Bearing:* why Forge should not promise binary Rust plugins: every plugin boundary would need
`repr(C)` or `abi_stable` types and could never be unloaded, which buys neither a sandbox nor hot
reload.

**DioxusLabs. `subsecond` 0.7.10; Bevy contributors, "Bevy 0.17" (30 September 2025);
`hot-lib-reloader` 0.8.2.** [code] [recent]
<https://docs.rs/subsecond/latest/subsecond/> — <https://bevy.org/news/bevy-0-17/> —
<https://docs.rs/hot-lib-reloader/latest/hot_lib_reloader/>

`subsecond` hot-patches a running Rust program: calls wrapped in `subsecond::call` go through a jump
table that a linker wrapper, driven by the Dioxus CLI, updates with newly compiled functions. Changed
struct layouts are unsafe, statics' destructors never run, thread-locals reset, only the main crate is
patched, and it is active only with debug assertions. Bevy 0.17 wired it to systems behind a
`hotpatching` feature: any system body can change while the game runs, but not its parameters, and
only in binary crates, not on WebAssembly. `hot-lib-reloader` is the older dylib approach:
`#[no_mangle]` functions in a reloaded library, fixed signatures and layouts, state carried by
serialisation, not for production.
*Bearing:* Rust code hot reload exists and is good enough for system bodies during development; Forge
can adopt `subsecond` as an optional dev feature of `forge-sim` without designing around it. Data hot
reload is the part that must be designed in.

**Epic Games. "Using Live Coding to Recompile Unreal Engine Applications at Runtime." Unreal Engine
5.8 documentation.** [docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/using-live-coding-to-recompile-unreal-engine-applications-at-runtime>

Live Coding patches rebuilt C++ into a running editor or game; it succeeds the older Hot Reload,
which remains only as a fallback.
Structural changes need object reinstancing, constructor defaults do not reach existing objects, code
holding pointers to reinstanced objects must fix them in reload callbacks, and it is unavailable on
consoles and mobile.
*Bearing:* the most-used C++ engine stops at the same line as `subsecond`: function bodies patch
well, type changes need reinstancing that the engine itself must support. It confirms "data first".

**The options side by side, for Forge.**

| | Sandbox | Deterministic across machines | Hot reload | Stable interface for outsiders | Cost to Forge |
|---|---|---|---|---|---|
| Rust, linked statically | none | same-binary only (D-016, `dmath`) | dev only (`subsecond`, system bodies) | none | none |
| Rust dylib plugins | none | same toolchain only | with state serialisation (Fyrox, `hot-lib-reloader`) | `repr(C)` / `abi_stable`, no unloading | high |
| Lua / Luau (`mlua`) | Luau's sandbox | only after replacing maths and iteration order (Factorio) | reload the script | the language plus Forge's bindings | medium |
| Rhai | yes, with limits | not established | reload the script | Forge's bindings | low |
| Wasm components (wasmtime) | yes, imports only, fuel | specified (Wasm 3.0 profile, wasmtime settings) | re-instantiate | WIT interfaces, versioned | medium–high |

---

## 4. How shipped games open themselves to mods

Every moddable game answers the same four questions: what is the unit a mod replaces (a file, a
record, a field), what happens when two mods touch the same unit, who decides the order, and how a
multiplayer session knows everyone has the same content. The answers below are ordered from the
coarsest to the finest.

**Bethesda plugin files. UESP, "Skyrim Mod:Mod File Format"; LOOT team, "Introduction To Load Orders"
and "How LOOT Sorts Plugins"; xEdit (TES5Edit, MPL-2.0).** [docs] [code] [still-current]
<https://en.uesp.net/wiki/Skyrim_Mod:Mod_File_Format> —
<https://loot.github.io/docs/help/Introduction-To-Load-Orders.html> —
<https://loot.readthedocs.io/en/latest/app/sorting.html> — <https://github.com/TES5Edit/TES5Edit>

A Creation Engine plugin (`.esm` master, `.esp`, `.esl` light) is a header listing its masters
followed by typed records; a record's FormID carries the load-order index of its master in the top
byte, which is how one plugin refers to — or overrides — another's records. When several plugins
contain the same record the last loaded wins outright, for almost every record type (LOOT's "rule of
one"), so compatibility is entirely a matter of order, with a limit of 255 full plugins plus up to
4,096 light ones in the newer games. LOOT orders plugins by building a graph of hard edges (masters,
metadata requirements), group rules and *overlap* edges from the records and assets plugins share,
then breaks ties towards the current order and sorts topologically; xEdit, covering Oblivion to
Starfield, is the community's record-level conflict viewer and editor.
*Bearing:* the negative lesson: whole-record override with last-wins forced a whole tool chain —
sorting, conflict viewing — to grow outside the game. Forge should resolve at *field* level and
report conflicts itself.

**Paradox Interactive / CK3 Wiki. "Modding"; "Mod structure".** [docs] [still-current]
<https://ck3.paradoxwikis.com/Modding> — <https://ck3.paradoxwikis.com/Mod_structure>

Crusader Kings III's content is script files in a directory tree, and a mod mirrors that tree. Mods
load from the top to the bottom of the launcher's playset; a file at the same path replaces the
earlier one entirely, and `replace_path` in `descriptor.mod` stops a whole vanilla folder loading;
single objects are overridden by redefining them in a file that sorts later — LIOS, "last in, only
served" — except UI types and templates, which are FIOS, "first in"; a few keys, such as events in
on-actions, append instead; conflicts are listed in `database_conflicts.log`. The descriptor carries
the mod's version and the game version it supports.
*Bearing:* three granularities — folder, file, object — and a documented merge rule per type are what
a content format needs. The part not to copy is order by convention (file names sorting): Forge's
order comes from manifests.

**Minecraft Wiki. "Data pack."** [docs] [still-current]
<https://minecraft.wiki/w/Data_pack>

The vanilla game's own features are defined by a built-in data pack, and data packs can redefine
advancements, dimensions, enchantments, loot tables, recipes, structures, biomes, functions, tags,
damage types and world-generation settings, all under namespaced ids (`namespace:path`); a
`pack.mcmeta` declares the pack format. When several packs provide the same file only the last one's
is used — except tags, which merge with earlier packs unless they set `"replace": true`; packs can
carry overlays; the order is set with `/datapack` and saved in the world. Data packs work on the
server side.
*Bearing:* the closest existing model to what Forge should do: the base game as a package, namespaced
ids, last-wins per record with an explicit merge rule for lists, the order saved with the world — and
content mods that need no code at all.

**NeoForged. "Registries" (NeoForge documentation) and "2023: The Good, The Bad… and The Fork"
(1 January 2024); FabricMC, "Fabric documentation"; SpongePowered, Mixin wiki.** [docs] [code]
[still-current]
<https://docs.neoforged.net/docs/concepts/registries/> —
<https://neoforged.net/news/2023-retrospection/> — <https://docs.fabricmc.net/develop/> —
<https://github.com/SpongePowered/Mixin/wiki/Introduction-to-Mixins---Understanding-Mixins>

Minecraft Java has no official code API. Fabric (loader, API and Gradle tooling) and NeoForge (the
fork of Minecraft Forge announced on 12 July 2023, joined by nearly all of Forge's team) load Java
mods that patch the game's bytecode as classes load, through Mixin: callback injections, redirects,
and overwrites that the Mixin docs single out as dangerous. NeoForge registries map namespaced names
to objects, must not be queried before registration finishes, and can sync *integer ids* to clients
for networking; data-pack registries load per world from JSON rather than at start-up.
*Bearing:* two lessons. Registries of namespaced names with per-session integer ids synced at
connection are what `MaterialId` should become. And when a game offers no code API, modders patch
bytecode and every mod couples to private internals — the argument for Forge to expose a small,
versioned API early rather than none.

**Factorio. "Data Lifecycle", "Libraries and functions", "Mod structure" (Lua API 2.1 documentation);
Factorio Wiki, "Multiplayer" and "Mod portal API".** [docs] [still-current]
<https://lua-api.factorio.com/latest/auxiliary/data-lifecycle.html> —
<https://lua-api.factorio.com/latest/auxiliary/libraries.html> —
<https://lua-api.factorio.com/latest/auxiliary/mod-structure.html> —
<https://wiki.factorio.com/Multiplayer> — <https://wiki.factorio.com/Mod_portal_API>

Factorio's mods are Lua. At start-up a settings stage and a prototype stage run every mod's
`data.lua`, then every mod's `data-updates.lua`, then every `data-final-fixes.lua`, so one mod can
change another's prototypes without depending on it, and the game records which mod changed which
prototype; within a round mods are ordered by the depth of their dependency chain, then by natural
sort of their names. At run time, `on_load` may only rebuild local references — anything else desyncs
multiplayer and replays — while migrations and `on_configuration_changed` handle version changes.
`info.json` declares the mod's version, the game version and dependencies, with prefixes for
incompatible (`!`), optional (`?`), hidden optional (`(?)`) and order-neutral (`~`) and comparison
operators. The Lua is 5.2.1 modified for determinism: no `io`, `os`, `coroutine`, `loadfile` or
`dofile`; `pairs` iterates in insertion order; `math.random` is the map's seeded generator; the
trigonometric, exponential and logarithmic functions are custom implementations. Multiplayer is
deterministic lockstep: every peer needs exactly the same game and mod versions, and mod checksums
are compared at join. The mod portal's API lists each release's version, required game version and
SHA-1.
*Bearing:* the case closest to Forge: a deterministic lockstep game with a large code-mod ecosystem.
What Forge's D-016 already does — no platform maths, seeded generators, ordered merges — Factorio had
to retrofit into Lua; the three data rounds and the dependency-depth order are a ready-made merge
order.

**RimWorld Wiki. "Modding Tutorials/PatchOperations"; Andreas Pardeike, Harmony 2.4 (MIT).** [docs]
[code] [still-current]
<https://rimworldwiki.com/wiki/Modding_Tutorials/PatchOperations> — <https://github.com/pardeike/Harmony>

RimWorld's content is XML Defs. Before Alpha 17 a mod could only overwrite whole Defs, and when two
did, only the last in load order survived. PatchOperations replaced that with XPath-targeted edits —
add, insert, remove, replace, set or remove attributes, conditional, sequence, find-mod — run after
all Defs load, in mod order, and before Def inheritance, so a patch to a parent reaches its children.
Code mods use Harmony, which patches .NET methods at run time with prefixes, postfixes, transpilers
and finalisers, several mods patching one method side by side; its README lists RimWorld, Stardew
Valley, Cities: Skylines and Kerbal Space Program among its users.
*Bearing:* RimWorld is the Bethesda lesson learnt in public — whole-record override does not survive
more than a few mods; targeted patches applied before inheritance do. Harmony shows that without an
API, modders patch methods.

**Roblox. "Client-server runtime." Roblox Creator documentation.** [docs] [still-current]
<https://create.roblox.com/docs/projects/client-server>

Every Roblox experience is user-made. The data model built in Studio becomes the runtime model on
Roblox's servers; clients receive copies and run their scripts locally; the server is the authority on
game state. Client changes do not replicate to the server by default: a client asks, and the server
decides.
*Bearing:* the model for untrusted gameplay code in a networked game: code that changes the shared
world runs on the server, clients run presentation, nothing a client sends is trusted. Forge's P8
(server authoritative, client predicts) extends to mod code the same way.

**Karel Moricky. "The Pandora's Box of Modding in 'Arma' Games." *GDC 2021*, Bohemia Interactive;
Bohemia Interactive, "Enfusion Workbench".** [talk] [web] [still-current]
<https://www.gdcvault.com/play/1027043/The-Pandora-s-Box-of> — <https://enfusionengine.com/workbench>

Two decades of Arma modding — which seeded DayZ and PUBG — presented as designing a game as a
platform: balancing openness against control, working with modders, and living with mods that become
more popular than the game. Bohemia's newer engine ships its development suite, the Enfusion
Workbench, to modders: the tools the studio uses, the Enforce Script language, workbench plug-ins, and
publishing to Bohemia's own Workshop for players on all supported platforms. Only the Vault abstract
of the talk is verified.
*Bearing:* the strategic reading of the owner's question: moddability is a product decision with a
long tail, and the studios that do it best give modders their own tools rather than a reduced kit.

**id Software. idStudio for DOOM Eternal, public beta announced 8 August 2024.** [web] [recent]
<https://idstudio.idsoftware.com/> —
<https://worthplaying.com/article/2024/8/8/news/143088-doom-eternal-finally-gets-official-mod-tools-public-beta-available/>
— <https://slayersclub.bethesda.net/en-EU/news/doom-eternal-pc-mods-update>

id released idStudio, the editor used to build DOOM Eternal, as a public beta, with a PC mod preview in
which players browse, download and play mods from the game's launcher, which manages a download queue
and the load order; the campaign's maps and DLC asset packs are provided as material. id's stated
reason is that the community had been modding the game without any official tools.
*Bearing:* even a closed, performance-first engine now ships its own editor to modders. An engine
whose tools live in the same crates as its runtime gets there far more cheaply.

**Godot Engine. "Exporting packs, patches, and mods." Godot documentation (stable, 4.7).** [docs]
[still-current]
<https://docs.godotengine.org/en/stable/tutorials/export/exporting_pcks.html>

Godot delivers DLC, patches and mods as resource packs (PCK or ZIP) loaded at run time. A pack's file
at an existing `res://` path replaces the original by default, so load order matters and packs can
patch earlier packs (the replacement can be turned off per load). The page names three security
failures — a mod carrying malicious code, a pack file swapped by malware, a compromised launcher —
and recommends signing packs with a private key checked against a public key in the main pack; it also
advises keeping mod tools separate from the game, which should not run a tools build of the engine.
*Bearing:* the minimal overlay design with its risks written down. Forge's packages are the same
overlay at record rather than file granularity; the signing advice applies to official packages and
to anything a server sends to clients.

---

## 5. Distribution and safety

**Valve. "Steam Workshop." Steamworks documentation; mod.io, "Documentation".** [docs]
[still-current]
<https://partner.steamgames.com/doc/features/workshop> — <https://docs.mod.io/>

The Workshop hosts user content and handles subscriptions, downloads and updates; a game integrates
through `ISteamUGC` (it reads the user's subscribed items and loads their folders) or leaves browsing
to Steam. A workshop is either ready-to-use (anyone uploads, no curation) or curated (the developer
approves every item; paid items with Steam handling payments and tax). mod.io sells the same service
as cross-platform middleware — PC, consoles, mobile, VR — with Unreal, Unity and C++ SDKs, a REST API
and moderation.
*Bearing:* distribution is a service to buy, not to build: the Workshop for a Steam-only release,
mod.io if consoles matter. What Forge must own is the package format and its hash, so that any of them
— or a game server — can carry it.

**fractureiser investigation team. "fractureiser" (GitHub), June 2023.** [web] [recent]
<https://github.com/fractureiser-investigation/fractureiser>

In June 2023 malware was found in Minecraft mods on CurseForge and BukkitDev, uploaded through
compromised developer accounts and pulled in as dependencies of popular modpacks; its later stages
stole credentials and copied themselves into other `.jar` files, on Windows and Linux. The community
investigation documented it publicly and met to discuss prevention.
*Bearing:* the price of native-code mods from an open portal: a mod is an executable, and one stolen
author account reaches every player of every pack that depends on it. Data-only packages cannot do
this, and Wasm mods can do only what their imports allow.

---

## Recommendation for Forge

**The short answer to the owner.** Yes — and the sources above point to one order: *data model
first, tools second, scripting last*. Forge is already half-way. P6 says rules are data, and D-007's
material table is the pattern every moddable game in §4 uses: a typed record per thing, referenced by
id, read by every system. What is missing is what lets such tables serve many projects and outside
authors: ids that do not depend on the order code runs in, files that layer, a merge that is
deterministic, and a hash that makes content part of the determinism contract. None of it needs a
scripting language, an editor or a mod portal; all of it is cheap now and expensive once content is
spread through demo code (RimWorld and Godot both paid for changing it late).

### Three layers

- **Engine** — the `forge-*` crates: mechanisms, and the record *types* they read (`Material` today;
  biomes, sound events, weather profiles, placement rules next). An engine crate never names a
  content id except reserved defaults such as `forge:default`.
- **Game** — a demo today; `tropical-island`, `world` and `shooter` in Phase 10: a crate that links
  the engine statically and registers its systems, record types and extension points through a plugin
  trait (Bevy's `Plugin`, The Machinery's registry). The engine never depends on a game; features
  attach to extension points, never the reverse (Unreal's Game Features).
- **Content** — *packages*: a directory (later an archive in the D-018 container) holding a manifest,
  RON record files and assets. The base game is a package like any other, as Minecraft's vanilla
  features are a built-in data pack; a mod is a package loaded after it.

### Identifiers (decide now)

| Kind | Form | Used for | Precedent |
|---|---|---|---|
| Authoring id | `package:path` string | references in files, saves, logs, network manifests | Minecraft resource locations, Unreal Primary Asset Ids |
| Runtime index | dense `u32` per table (`MaterialId` today) | GPU rows, per-triangle ids, hot loops | NeoForge's synced integer ids |
| Content hash | BLAKE3 of cooked bytes (D-018) | caching, deduplication, fetching by hash | the D-018 container |

Rules:
- Runtime indices are assigned after the merge — reserved engine rows first, then the rest sorted by
  id — so they depend only on the *set* of content, never on file-system, thread or load order
  (Unity's baking caveat).
- Indices are never written to saves, and never sent without the manifest that produced them.
- Ids are logical names, not paths, so files move freely without sidecars (the Godot and Unity
  lessons).
- UUIDs for editor-placed instances are a question for the editor, not for now.

### Records and schemas (decide now)

- A record type is a Rust struct with `serde` derives and `#[serde(deny_unknown_fields)]` (audio.md
  already asks for this), and every field added after the first release has a default. Fields carry
  doc comments (Bilas' schema docs) that a tool can extract later.
- Each table has a schema version. The loader runs migrations from the file's version to the current
  one (Blender's `do_versions`); renames go through serde aliases; a removed name is never reused
  (protobuf's reservation rule, Verse's publication rules).
- **Three schemas, kept apart** (Chabant): the RON file is the storage schema — persistent, versioned,
  edited by people; the runtime table (D-026's GPU row, `slotmap` pools) may change at every
  release; the editor's view comes later. A GPU row never appears in a file.
- RON is the text format; `rkyv` only for cooked tables, and only if loading ever shows in a profile.
  Reflection (`bevy_reflect`, or `facet` if it matures) arrives with the inspector, not before.

### Packages and the merge (Phase 2)

```ron
// packages/icier-ice/package.ron
Package(
    name: "icier-ice",
    version: "1.2.0",
    forge_data: 1,                        // the record-schema generation it was written for
    depends: ["asteroids >=0.3, <0.4"],
    after: ["?better-rock"],              // optional ordering hint, as Factorio's "?"
)

// packages/icier-ice/materials.ron
[
    Add("icier-ice:blue-ice", Material(name: "blue ice", render: (class: Ice /* … */))),
    Patch("asteroids:ice", { "render.scattering": 0.4 }),   // sparse override: USD's `over`
]
```

The merge, in order:
1. Resolve the package order: dependencies first, by dependency depth, then by name (Factorio's
   rule), adjusted by `after` hints; a cycle is an error.
2. Apply each package's operations per table: `Add` (the id must not exist yet), `Replace` (the whole
   record; last wins, the rule of one), `Patch` (named fields only; list fields take `add`/`remove`,
   as The Machinery's sets and Minecraft's tags do).
3. Log every field written by more than one package to a conflict report (CK3's
   `database_conflicts.log`, what xEdit shows).
4. Validate every reference: the id exists, in the right table. Errors happen at load, never at use.
5. Freeze the registries (NeoForge), assign indices, build the runtime tables.

In development, a file change re-runs the merge. If the id set is unchanged, tables update in place
(the material table is re-uploaded); otherwise the dependent tables are rebuilt.

### Determinism under mods (Phases 3 and 5)

- The merge is a pure function of the package set. Its **manifest hash** — BLAKE3 over the resolved
  order of package names, versions and content hashes — joins D-016's CI digests and D-010's
  handshake. A client with a different hash is refused, as Factorio refuses mismatched mods; later it
  fetches the missing packages by hash from the server, as Veloren does.
- Anything evaluated on both sides from data (procedural rules, weather profiles, material-state
  thresholds) goes through engine evaluators that use `forge_core::dmath`; data cannot bring its own
  maths.
- Saves store ids, never indices.

### Game code and mod code (Phase 10, sketched now so nothing blocks it)

- Engine and games link statically. **No binary Rust plugins**: `repr(Rust)` has no layout
  guarantee, `abi_stable` cannot unload, and even Godot's C ABI broke between minor versions.
- Iteration in development: data hot reload first (above); shaders already reload; `subsecond`
  hot-patching of system bodies as an optional dev feature of `forge-sim` (Bevy 0.17 shows it works
  for systems) — useful, never a design constraint.
- **Mod code, if the owner wants it**, runs as WebAssembly components in wasmtime:
  - sandboxed, reaching the engine only through a WIT API versioned by semver per interface (The
    Machinery's rules) and checked for compatibility when a package is published (Verse's);
  - deterministic settings: NaN canonicalisation, deterministic relaxed SIMD, fuel as the per-tick
    budget;
  - server-side by default, with results replicated (Roblox's model); client-side only for
    presentation.
- Luau through `mlua` is the alternative if designers want a scripting language for UI and tools,
  and its sandbox list is the checklist. Native-code mods are ruled out (fractureiser; Quake III's
  reasons for its VM).

### Decide now, decide later

| Item | When | Why then |
|---|---|---|
| Ids: `package:path`, dense indices from the sorted set, indices never persisted | now | `MaterialId` exists; every table written after this inherits the scheme |
| Records: serde + RON, unknown fields rejected, defaults, schema version and migrations | now | biomes (D-034), tone curves (D-022) and audio events are the next tables |
| Packages, the ordered merge, patches, conflict report, reference validation | Phase 2 (`forge-data`) | cheap before content spreads; RimWorld shows the cost of retrofitting |
| Manifest hash in the digests and the handshake | Phases 3 and 5 | D-016, D-010 |
| Game crates as plugins | Phase 10 | the second game is when the seam is really tested |
| Reflection for the editor (`bevy_reflect` or `facet`) | with the editor | the inspector needs it; loading does not |
| Mod code (Wasm) or a scripting language | after Phase 10, if wanted | depends on the games; nothing above blocks it |
| Distribution (Workshop, mod.io, own server) and package signing | at ship | a service, not engine code |

### How it fits the roadmap

- **Phase 2:** a small `forge-data` crate — ids, registries, RON loading, packages, the merge, the
  conflict report, validation, the manifest hash; 1.5–2.5 k lines is an estimate, not a measurement.
  First move: the stock materials (rock, ice, the city's rows) leave demo code for
  `packages/forge/materials.ron` and each demo's own package; D-034's biome envelopes are the second
  table.
- **Phase 3:** simulation, physics and material-state tables load from packages; the determinism
  digest includes the manifest hash.
- **Phase 5:** the handshake exchanges manifests; content is fetched by hash.
- **Phase 6:** audio events and buses are records, as audio.md already specifies.
- **Phase 10:** the games become plugin crates; the first "mod" is a package that re-materials and
  re-biomes the island without code; scripting and mod code are decided then.
- **Editor:** its own research file (RESEARCH.md still lists it). It edits these records, and The
  Truth's change-record model (procedural.md §6) sits on top for undo.

**The demo that proves it.** `city-blocks` loads its materials from packages. A second run adds a
package that patches the glass's reflectance and adds one material: the captures differ only on the
patched surfaces, the conflict report lists the patched fields, the printed manifest hash changes, and
the run without the extra package is pixel-identical to today's golden image (D-017).

---

## Checked and left out

Kept, as in the other files, so the bibliography is auditable.

- **Frostbite's "A Tale of Three Data Schemas" beyond the abstract.** EA's own page
  (`ea.com/frostbite/news/a-tale-of-three-data-schemas`) returned 404 directly and through a reader
  proxy; the GameDev.net write-up returned 403 and an empty page through the proxy; a third-party
  notes page returned no body. Only the GDC Vault abstract is used; claims about which schema is
  persistent are Forge's reading, not quoted from the talk.
- **Guerrilla's GDC 2017 slides.** The PDF exceeds the fetch tool's 10 MB limit and the proxy returned
  nothing; only the publication page is cited. Nothing about Decima's type system, file format or
  editor internals is claimed.
- **Bethesda's own Creation Kit wiki.** `ck.uesp.net` served a bot-check page (not bypassed) and
  `creationkit.com` redirected to a maintenance page on `wiki.bethesda.net`. The UESP main wiki, the
  LOOT documentation and the xEdit repository carry the facts instead. Nothing about Starfield's
  Creation Kit or Bethesda's paid Creations is claimed.
- **"Passing a Language through the Eye of a Needle" (Ierusalimschy et al., ACM Queue 2011).**
  queue.acm.org served a bot-check page; not bypassed. The 2018 CACM paper by the same authors, read
  through a reader proxy, is cited instead.
- **Liebgold's CUFP 2011 talk ("Functional mzScheme DSLs in Game Development").** `cufp.org` failed
  with a self-signed certificate; the GDC 2008 talk, on archive.org, is cited instead. That Naughty Dog
  kept Racket DSLs after the PS3 came from search results only and is not claimed.
- **O3DE's "Versioning Serialized Data" page.** The pages render mostly navigation; the proxy render of
  the serialisation page showed no version converters, so O3DE's data-migration mechanism is not
  described.
- **Arma Reforger's FAQ answers** on console mods and Workshop — only the questions rendered, directly
  and through the proxy. The cross-platform claim rests on the Enfusion Workbench page's wording; a
  PCGamesN article on PC/Xbox mod compatibility surfaced in search and was not fetched.
- **Minecraft pack-format minor versions.** The wiki render mentioned them, but the version numbers
  could not be pinned down; only the existence of `pack.mcmeta`'s pack format is stated.
- **Fabric's mappings (Yarn, intermediary).** Not on the page fetched; not claimed.
- **Veloren's current plugin API.** The book's page describes the older `wasm32-wasi` toolchain; a
  merge request and search results suggest a newer design, not confirmed. The entry says so.
- **Bevy's BSN pull request** (#23413, reported merged 27 March 2026) came from search results only;
  the 0.19 release post is cited instead.
- **mod.io and consoles.** Whether consoles allow mods containing code is not in mod.io's
  documentation; not claimed.
- **A Godot security warning about `.tres` files running scripts.** The fetch tool's summary of the
  Resources page reported one; the page's source on GitHub shows the warning is about script *inner
  classes* not serialising, not about security. The claim was dropped; Godot's real security guidance
  is on the packs-and-mods page cited in §4.
- **Unity Addressables for mods and DLC.** The manual's front page does not describe loading extra
  catalogs at run time; not claimed.
- **Paradox's mod platform (Paradox Mods) and multiplayer checksums** — not researched.
- **Stardew Valley's Content Patcher, Kerbal Space Program's ModuleManager, Nexus Mods and Vortex** —
  not researched; RimWorld's patches and the Bethesda tools cover the same ground.

---

## Verification notes

- **Method.** WebSearch and WebFetch only; **no browser pane or browser automation was opened at any
  point**. Where a documentation page was client-rendered or refused the fetch tool (Unreal's blog and
  documentation, the CACM paper, O3DE's serialisation page, LOOT's introduction), it was read through
  the public reader proxy `r.jina.ai`. The Verse book's chapter list came from the GitHub contents API and the "Code Evolution"
  chapter from its raw Markdown on GitHub; Godot's pages were checked against their `.rst` sources in
  the `godot-docs` repository.
- **Summaries checked against sources.** The fetch tool summarises pages with a small model, and twice
  it misreported: it invented quotations for Bilas' 2002 slides (the PDF was then saved and its text
  extracted with `pdftotext`; the entry uses only that text), and it turned a Godot serialisation
  warning into a security warning (see *Checked and left out*). Entries whose wording matters (Verse's
  rules, Godot's packs, Bilas) were read from the primary text.
- **Bot checks not bypassed.** `queue.acm.org` and `ck.uesp.net` presented bot-verification pages;
  both were left alone and replaced by other sources.
- **Versions** are as displayed on 25 September 2026: `bevy_reflect`/`bevy_asset`/`bevy_app` 0.19.1,
  `facet` 0.46.5, `rkyv` 0.8.18, `ron` 0.12, `mlua` 0.12.1, `rhai` 1.26.1, `wasmtime` 49.0.1,
  `abi_stable` 0.11.3, `subsecond` 0.7.10, `hot-lib-reloader` 0.8.2, Harmony 2.4, Unreal Engine 5.8
  documentation, Unity 6.2 manual, Godot stable documentation 4.7, Factorio Lua API 2.1.x. They will
  drift.
- **Claims made without a primary source.** The line-count estimate for `forge-data`, the demo design
  and the table of trade-offs in §3 are Forge's judgement from the entries, not measurements. "Not
  established" for Rhai's determinism means no statement either way was found.
- **Date.** Checked 25 September 2026.
