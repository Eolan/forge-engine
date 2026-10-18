# Missions as data: quests, objectives and triggers in shipped games

Research for issue #287 and D-072 (2026-10-18). Missions are data first: a `Mission` record in Forge's content packages (D-035), evaluated by a Rust system at the fixed tick inside the predicted, digested simulation (D-010, D-016). Luau comes later and never runs inside the predicted step. The village generator (D-062) will write mission records for generated maps.

This note goes with three others. [scripting.md](scripting.md) covers why data comes first, Verse's operators and Luau. [data-driven.md](data-driven.md) covers packages, patches and migrations. [netcode.md](netcode.md) covers Destiny's mission hosting and prediction. Numbers in brackets refer to the source list at the end. Pages were read on 2026-10-18. Where only a search summary could be read, the text says so (see "Checked and not confirmed"). None of this is legal advice.

## Summary

- **Shipped systems split a mission into the same parts:**
  - **a definition kept apart from its state.** Definitions are stages, objectives, a graph or a state machine. State is stage flags, facts and counters.
  - **named roles the definition refers to,** bound to world entities when the mission starts. Examples: Bethesda's aliases, Hitman's repository ids, Paradox's saved scopes, Daggerfall's resources.
  - **a small vocabulary of conditions:** in an area, near, holds an item, killed, a timer, a counter, and/or/not.
  - **objectives as the player-facing view.** They are often separate from the internal stages.
- **Flow.** Verse's operators match what quest graphs already do:
  - CD Projekt RED's *And* node is `sync`.
  - Its *Xor* node is a race whose losers are not cancelled.
  - Its *Cut Control* node is cancellation.
  - Naughty Dog's concurrent *tracks* are branches inside a state.
  - StateTree runs every task of the active states at once.
- **Cleanup is scoped.** A losing arm is cancelled. A phase's other nodes stop when its output fires. Bethesda's alias data drops when the quest stops. Naughty Dog runs an exit block on a transition.
- **Saves and patches work best with small integer state keyed by stable ids.**
  - CD Projekt RED leaves a *deletion marker* when a node is removed. The signal "can't die when the quest is updated in a patch" [15].
  - Factorio runs each migration once per save and records which ones ran [37].
  - Bethesda's saves are the counter-example: modders warn against updating running quests on existing saves [8].
- **Determinism.**
  - Factorio's scenario logic runs in lockstep. It compares a cheap checksum every tick, and a full-map checksum in a debug mode finds the first tick that diverges [37][38].
  - Photon Quantum raises events on predicted frames. It removes duplicates by a hash that includes the tick, and confirms or cancels each event when the server's input arrives [49].
- **Proposal (§7):**
  - steps in a tree with Verse's operators (sequence, `sync`, `race`, `branch`);
  - leaf steps that wait on *level-triggered* conditions (states, not one-off events);
  - roles bound once and saved as stable ids;
  - objectives derived from step state;
  - a sparse step-state map that is saved and digested;
  - one *reconcile* rule for both hot reload and old saves.
- **Patents (§6).** Nothing found clearly reads on a plain record and evaluator. Two families should be read before particular moves:
  - Zynga's state-machine-scripting patents, before a server sends mission definitions to clients at run time or records drive objects' visuals;
  - Kabam's "dynamically generated in-game objectives", before the generator picks objective types, amounts or time limits at random.

**Contents**
1. [The survey](#1-the-survey)
2. [Shared structures](#2-shared-structures)
3. [Concurrency, cancellation and ordering](#3-concurrency-cancellation-and-ordering)
4. [Saves and hot reload](#4-saves-and-hot-reload)
5. [Determinism and networking](#5-determinism-and-networking)
6. [Patents](#6-patents)
7. [Proposed schema for Forge's first version (proposal)](#7-proposed-schema-for-forges-first-version-proposal)
8. [Sources](#sources)
9. [Checked and not confirmed](#checked-and-not-confirmed)

---

## 1. The survey

| System | Unit of definition | State kept | How it refers to entities | Flow |
|---|---|---|---|---|
| Bethesda Creation Kit | quest: stages, objectives, aliases | stages done, objectives shown/completed/failed, alias fills, script variables | aliases filled at quest start | stage numbers set by script fragments |
| Paradox (CK3, Stellaris, EU4) | events (trigger, immediate, options); mission trees; story cycles | variables, flags, a story object's variables | scopes: `root`, saved `scope:name` | event chains, a DAG of missions, periodic effect groups |
| CD Projekt RED REDkit / REDengine 4 | quest graph of phases and nodes | facts (named signed integers) and where the signals are | tags | signal flow: Pause, Condition, And, Xor, Cut Control |
| Guerrilla (Horizon Zero Dawn) | quest graph of steps, each a verb and a target | which steps are done | the step's target | completing a step opens or closes parts of the graph |
| IOI Hitman (Glacier 2) | objectives as JSON state machines over game events | the state name and a context of variables, per scope | repository ids | states, event handlers, `$timer`, transitions |
| Bungie Destiny | objective definitions inside quest items | a progress value per objective | location hashes | quest steps |
| Naughty Dog | state scripts (Scheme) | the current state | attached object, regions | states, handlers, concurrent tracks |
| Unreal StateTree / Verse | hierarchical states and tasks / code | the active states / a suspended task | bound context data | transitions; Verse's sync, race, rush, branch |
| Unity (Chop Chop) / Godot (Questify) | ScriptableObjects / a graph resource | `_isDone` on the asset / serialised quest state | actor and item assets / conditions queried by signal | ordered steps / graph with Any and Exclusive branches |
| Factorio | scenario Lua | the `storage` table | entity references in storage | events |
| Arma (2, 3) | editor triggers, waypoints, tasks | trigger state, task states | object variables | waypoint order and synchronisation; task hierarchy |
| BioWare Mass Effect | plot variables, conditionals, transitions | bools, ints, floats by index | — | conditionals gate dialogue and plot |
| Daggerfall (1996) | quest scripts: resources and tasks | task flags, clocks | typed resources (Person, Place, Item, Clock) | tasks fire when conditions hold |

### 1.1 Bethesda's Creation Kit (Skyrim, Fallout 4)

- **Stages.**
  - A stage is an integer index: 0–65535 in Fallout 4's editor, 0–255 in the Fallout 3 and Oblivion editors [2].
  - Community guidance is to leave gaps of 5–10 between numbers so stages can be inserted later [2].
  - A stage can carry log entries, a *Complete Quest* or *Fail Quest* flag, and a script fragment that runs when the stage is set [1][2].
  - The quest list shows only the latest log entry [1].
- **Objectives.**
  - Objectives are a separate list, by index, each with display text and targets.
  - A target is an alias plus conditions. Displayed targets that pass their conditions appear on the compass and map [3].
  - *ORed With Previous* merges two objectives into one "A or B" line. Completing either one completes the group [3].
  - Scripts show, complete and fail objectives (`SetObjectiveDisplayed` and related functions), usually from stage fragments [3].
- **Aliases** are named roles.
  - Fill types include a specific reference, a unique actor, the first reference matching conditions, a newly created reference, another quest's alias, and data from the event that started the quest [4][6].
  - If an alias that is not *Optional* cannot be filled, the quest fails to start [4].
  - A reference held by a *Reserved* alias is not given to other aliases unless they allow it [4].
  - Aliases fill once, at quest start, in list order. One failed fill can stop the aliases below it from filling (community reports) [7].
- **Alias data is scoped.**
  - Spells, factions and AI packages apply only while the quest runs and the reference stays in the alias [5].
  - Items given through an alias stay in the inventory afterwards [5].
- **Story Manager.**
  - A world event, such as a kill with its victim and killer, walks down a tree of nodes with conditions and starts the first valid quest [6].
  - That quest can fill its aliases from the event's data [4][6].
- **Saves.**
  - Skyrim's save stores *change forms*: deltas against the base records, with flags saying which parts changed [9]. Skyrim's quest change form was not found documented; Oblivion's stores stage done-flags [9].
  - Modders warn against updating a start-game-enabled quest's scripts on an existing save [8].
- *Bearing:* copy the roles that are bound once with an *optional* flag and a rule for when binding fails. Scoped alias data shows that cleanup can be automatic. Showing objectives from scripts is flexible but drifts away from the stages: derive objectives from step state instead.

### 1.2 Paradox (Crusader Kings III, Stellaris, Europa Universalis IV)

- **Events** [10][13]:
  - ids are `namespace.id`;
  - a `trigger` block holds conditions, `immediate` holds effects, options hold effects and `after` runs once an option is chosen;
  - the event's `type` sets its root scope;
  - triggers and effects are strictly separate.
- **Scopes** [11]:
  - `root`, `this` and `prev` (one step back only);
  - `save_scope_as` names a scope, read as `scope:name`;
  - a saved scope lasts through an unbroken chain of effects, so an event can pass it to the events it fires;
  - list iterators: `any_` (a trigger, with a count), `every_`, `random_`, `ordered_`.
- **Firing** [10][13]:
  - code fires *on_actions*;
  - Stellaris checks every event's triggers against each object at least once a game day;
  - `mean_time_to_happen` with modifiers checks every day for each scope, which the wiki calls considerably more expensive;
  - `is_triggered_only` removes an event from polling;
  - delayed calls (for example 200 days plus up to 100 random) re-check their conditions when they run.
- **EU4 mission trees** [14]:
  - missions sit in columns ("series");
  - `required_missions` makes them a graph without cycles;
  - `trigger` decides completion and `effect` runs when the mission is claimed;
  - `completed_by` completes a mission on a date.
- **CK3 story cycles** [12] are background processes with state:
  - an owner, `on_setup`, `on_end` and `on_owner_death`;
  - effect groups that fire every given number of days or months, with a chance and triggered effects (`first_valid` runs the first one that passes);
  - variables that outlive the owner;
  - `end_story` runs `on_end`.
- *Bearing:* the split between triggers and effects and the named scopes are the right shape. Stellaris's polling cost argues for evaluating only active steps. A story cycle is a `branch` with its own cleanup.

### 1.3 CD Projekt RED: REDkit (The Witcher 3) and REDengine 4 (Cyberpunk 2077)

- **The quest graph**, from CD Projekt RED's own REDkit documentation [15]:
  - *Phases* are subgraphs with input and output sockets. When the signal passes a phase's output, every other node inside the phase stops.
  - *Pause* holds the signal until all its conditions are met. The documentation calls it one of the most important nodes.
  - *Condition* sends the signal out of a true or false socket at once.
  - *And* releases the signal when all its inputs have received one.
  - *Xor* passes the first input and then blocks the node for good.
  - *Cut Control* kills the signal in a connected Pause or Phase. Its *permanent* flag stops them from activating again.
  - Other nodes: *Random*; *FactsDB Change*; *Journal entry*; *Objective Counter* (increment, decrement, reset); *Checkpoint*; *Scene*; *Script*.
  - A phase's `saveMode` decides whether the player may save while the signal is inside it.
  - A *deletion marker* replaces a deleted node so that a signal in a saved game survives a patch.
- **Conditions and functions** [16]:
  - actor conditions (health, state, weapon drawn);
  - fight conditions (killed, stunned, hit by a given sign);
  - scripted conditions (item quantity, item equipped, container, book read);
  - functions such as adding and removing items, changing a door's state, sounds and fades.
- **Entities** are referred to by tags. Journal quests and objectives are separate entries that graph nodes activate, succeed or fail [16].
- **Cyberpunk 2077's facts** [17]:
  - a fact is a name and a signed integer, read as 0 until it is set;
  - tests use `<`, `<=`, `==`, `>=` and `>`, and a test can form a Pause or a Condition;
  - the save tracks progress through facts;
  - the modding wiki warns that editing facts by hand tends to cause problems much later in the game.
- *Bearing:* the graph passes a token around, like a Petri net: And is a join, Xor takes the first arrival, Cut Control removes a token. The deletion marker is the stable-id and tombstone rule Forge needs.

### 1.4 Guerrilla Games: Horizon Zero Dawn

Szczepanski (GDC 2017) places quest systems on a scale [18]:
- **relaxed:** no central manager, everything in scripts, any story possible, each quest costly to build;
- **strict:** a central system with predefined elements, no scripting, faster to build and easier to discuss.

Horizon sits between the two.
- A quest is a graph of steps linked by cause and effect.
- Each step is a verb from a finite list (obtain, kill, converse and others) plus a target.
- Completing a step opens or closes parts of the graph.
- Almost 150 quests were built this way, and simple fetch quests were automated.
- Two limits are reported: the system could not *revert* events when progress had to be shown again (their "time travel problem"), and the verb list was too small.

### 1.5 Ubisoft and Massive (Dunia, Snowdrop)

- No public description of their mission data model was found.
- Massive described Snowdrop at GDC 2014 as a node-based system that also drives mission scripting, chosen so that ideas need less programming [19].
- Watch Dogs: Legion's missions and its "Play As Anyone" design were a GDC 2021 talk. Only the abstract was found [20].

### 1.6 IO Interactive: Hitman (Glacier 2)

The Peacock project, a server replacement for the HITMAN trilogy, mirrors the contract data the official servers send [21]. It also reimplements the engine's state machines [22].
- **An objective** has:
  - an id, a category (primary, secondary, condition), hidden and scoring flags, and a HUD template;
  - either a success event (for example *Kill* with the target's repository id), or a *statemachine* definition.
- **A state machine definition** has [21][22]:
  - a context (variables) and constants;
  - named states, each mapping event names to handlers;
  - optional conditions on a handler: `$eq`, `$and`, `$or`, `$not`, `$any`, `$all`, `$inarray`, `$after`;
  - actions: `$inc`, `$dec`, `$set`, `$push`, `$pushunique`, `$remove`, `$reset`;
  - a transition;
  - *Success* and *Failure* as the outcomes;
  - a special handler run on entering a state;
  - `$timer` handlers.
- **Example.** In the 2019 "Gecko" contract, a target's escape is a state that waits for an escape event and then races a 20-second timer against the kill or a foiling event [21].
- **Semantics, as Peacock reimplements them (not IOI documentation)** [22]:
  - timer handlers are evaluated before the event;
  - handlers run in their listed order, and the first transition ends processing;
  - a transition resets the timers and runs the new state's entry handler;
  - if a timer causes a transition, the current event is replayed against the new state.
- **Where state lives** [21]: *profile* (persistent), *hit* (per contract) or *session* (lost on restart).
- *Bearing:* this is the closest shipped precedent for objectives as small data-defined state machines driven by events. Its ordering rules are a model to state explicitly.

### 1.7 Bungie: Halo 3 and Destiny

- **Halo 3's AI objectives** (Isla, GDC 2008): designers place tasks in the environment, and the system distributes squads across them [23]. Only the abstract was read. This is encounter data, not player missions.
- **Destiny's objective definition** (Bungie.net API) [24] shows progress semantics:
  - `completionValue`, and `isCountingDownward` (complete when the value falls to the target);
  - `allowNegativeValue`;
  - `allowValueChangeWhenCompleted` (progress can un-complete an objective);
  - `allowOvercompletion`;
  - `minimumVisibilityThreshold` (progress hidden below a value);
  - an optional location;
  - separate display styles for in progress and completed.
- **Destiny's missions** are simulated on a server host, with player state peer to peer [25] (netcode.md).

### 1.8 Naughty Dog: state scripts (Uncharted)

Gregory's GDC 2009 slides describe them [26]:
- a script has attributes and states;
- a state has handlers for begin, update (every frame), end and named events;
- a handler holds one or more *tracks*, which run concurrently like fibers, each until a `wait` (seconds, an action finishing, a signal from another track);
- `go` changes state and aborts the running blocks;
- the slides advise an exit block for cleanup.

The system grew out of in-game cinematics, and data and scripts share one Scheme-based toolchain [26].

### 1.9 Unreal: StateTree, Gameplay Tags, Gameplay Messages and Verse

- **StateTree** [27][28]:
  - a hierarchical state machine;
  - selecting a leaf activates it and its parents, and the tasks of every active state run concurrently;
  - transitions fire on completion, success, failure, a condition checked every tick, or an event;
  - transitions are evaluated from the leaf upward, and the first that succeeds wins.
- **How a state completes** [28]:
  - since UE 5.6 a state's *TasksCompletion* is set to *Any* or *All*, counting only the tasks marked to take part;
  - before 5.6, any task that finished completed its state.
- **Composition** [28]:
  - a state can run another StateTree asset as a *linked asset*;
  - *Run Parallel Tree* runs another tree beside it, at a cost.
- **Events** [28]:
  - an event is a gameplay tag, an optional struct payload and an optional origin;
  - a transition can consume its event so that later selection does not see it.
- **An ordering caveat** [29]: a forum report says that when two events arrive in the same frame and trigger transitions on different states, the deeper state's transition wins and the parent's is dropped.
- **Gameplay Tags** [30] are hierarchical names (`A.B.C`). A query can match the exact tag or any of its parents.
- **The Gameplay Message Subsystem** (Lyra) [31] broadcasts struct messages on tag channels to listeners. Broadcasts are local and not replicated.
- **Verse** [32]:
  - `sync` waits for every arm and returns their results in source order;
  - `race` starts the arms in source order, the first to finish wins and the rest are cancelled. An arm the race resolved before it started never runs;
  - `rush` takes the first but lets the rest run until they finish or the enclosing context ends;
  - `branch` runs in the background and is cancelled when its enclosing scope is left;
  - `spawn` can outlive its creator;
  - cancellation is cooperative (at suspension points), cascades from parent to child, and runs `defer` blocks.
  - A forum post describes Verse as server-authoritative, with client prediction hidden from Verse code ("Predicts", in progress) [33].

### 1.10 Unity and Godot

- **Unity's Open Project 1 ("Chop Chop")** [34]:
  - a questline holds quests, and a quest holds steps (Dialogue, GiveItem, CheckItem) with an actor, an item, a reward and an end event;
  - the step asset also serialises its own `_isDone` flag, so the definition and the runtime state share one object. This is the pattern to avoid.
- **Pixel Crushers' Quest Machine** [36]: a node editor, counters (its quick start uses "Counter coins >= 3"), custom conditions, and quests generated from NPCs' goals.
- **Questify for Godot 4** [35]:
  - a graph resource with one start and one end;
  - objective nodes (with an optional flag and metadata for markers);
  - Condition, Any Condition and Not;
  - *Any Previous* and *Exclusive Branch*;
  - conditions are queried by signal and polled every 0.5 s by default;
  - `serialize()` and `deserialize()` save the state, and quests must be re-instantiated from the original resource.

### 1.11 Factorio

- Scenario and mod logic is Lua, run in lockstep on every peer [37].
- State lives in the `storage` table [37].
- `on_load` may read `storage` but not write it. The documentation says anything else there leads to desyncs [37].
- `on_configuration_changed` runs when a version or the mod set changes [37].
- **Migrations** [37]:
  - each runs once per save, and the save records which ones ran;
  - JSON migrations rename prototypes, then Lua migrations fix state.
- **Event order** is set by dependency depth, then mod name [37].
- **Desync detection** [38]:
  - a cheap checksum of a few values is compared every tick (FFF #55);
  - a debug mode checksums the whole map every tick, so a replay finds the first divergent tick (FFF #47);
  - a desync makes the player rejoin and download the map (FFF #188).

### 1.12 Arma 2 and 3 (Bohemia Interactive)

- **Triggers** [39][40]:
  - an area, an activation (a side present or absent, "detected by", radio), a condition expression, a timer (after a delay, or while the condition holds), on-activation and on-deactivation statements, *repeatable* and *server only*;
  - conditions are checked about every half second (a user note on the BI wiki), and the interval can be set per trigger;
  - a tracker report says non-repeatable triggers keep evaluating after they fire.
- **Waypoints** are ordered per group. Synchronisation links waypoints and triggers: an AND waypoint waits for all its synced waypoints, an OR waypoint for any. A 2014 tracker report says OR fires at once in Arma 3 [40].
- **The task framework** [41]:
  - states CREATED, ASSIGNED, SUCCEEDED, FAILED, CANCELED;
  - parent and child tasks;
  - SUCCEEDED, FAILED and CANCELED all count as completed.
- **Generated missions** [42]:
  - Arma 2's *Secondary Operations Manager* starts random side missions from a pool, with an activation chance and spacing rules;
  - Arma 3's *Combat Patrol* builds short randomised scenarios at a location the players choose.

### 1.13 BioWare: Mass Effect's plot manager

Mass Effect keeps global plot variables (bools, ints and floats, by index, false or 0 by default) [43]. *Conditionals* combine them and are referred to by index. *Transitions* are rules that change them. It is the same idea as CD Projekt RED's facts, with named combinations.

### 1.14 Daggerfall (1996)

A quest's source declares typed *resources* [44]:
- Person;
- Place (a permanent site, or one picked at run time, local or remote);
- Item;
- Clock.

Tasks fire their actions when conditions hold. A Clock can be sized from the travel distance to a Place [44]. This is role binding for generated content, thirty years before Forge's generator.

### 1.15 Research on generated quests

- **Doran and Parberry (PCG 2011)** [45]:
  - they analysed more than 750 quests from four MMORPGs and found a common structure;
  - an NPC motivation picks a strategy, a sequence of actions;
  - actions such as *goto*, *get*, *kill*, *steal* and *capture* expand recursively into sub-strategies, down to a depth limit.
  - A third-party reimplementation lists nine motivations (knowledge, comfort, reputation, serenity, protection, conquest, wealth, ability, equipment). Its terminal actions include *give*, *report*, *use*, *damage*, *escort*, *defend* and *exchange* [45].
- **Dormans (PCG 2010)** [46]:
  - generates a *mission* first, as a graph grammar of tasks, then a *space* that fits it;
  - *Unexplored* keeps locks and keys abstract until late;
  - the key–lock links survive every rewrite, and the game uses them to find and fix blockers.
- **Petri nets** [48]:
  - used to validate missions (Vega, Grünvogel and Natkin, 2004);
  - to model games for formal analysis (Araújo and Roque, DiGRA 2009);
  - to prove quests sound as workflow nets (Oliveira, 2012).
- *Bearing for the generator:* emit records in a small action vocabulary that maps onto Forge's triggers. Keep the generator's dependency graph (which item or key unlocks what) so a validator can prove the mission can be completed.

---

## 2. Shared structures

| Concept | Bethesda | CD Projekt RED | Hitman | Paradox | Arma | Proposed for Forge v1 |
|---|---|---|---|---|---|---|
| Internal progress | stage numbers | where the signals are, plus facts | state name, plus context | variables, flags | trigger states | step states, by step id |
| Player-facing goal | objectives (separate) | journal objectives (separate) | objectives (one each) | mission and decision texts | tasks | objectives derived from steps |
| Entity references | aliases, filled at start | tags | repository ids | scopes | variable names | roles, bound at start |
| Conditions | condition functions | Pause/Condition conditions, facts | `$eq` and operators on event values | triggers | condition expressions | a fixed set of state predicates |
| Counters | script variables | facts, Objective Counter | `$inc` on the context | variables | variables | integer vars, `CountOf` |
| Failure | Fail Quest stage | journal *fail* | *Failure* state | — | FAILED and CANCELED tasks | step fails; mission fails; cancelled is separate |
| Optional or hidden | objective display | journal | IsHidden, Category | — | — | per objective |
| Timers | script timers | Pause on time | `$timer` with `$after` | delays, MTTH | trigger timeout | `After`, from the step's start |
| Rewards, consequences | stage fragments | FactsDB, scripts | scoring | effects | statements | actions; rewards as events |
| UI and audio events | script calls | journal notices | HUD templates | — | task notifications | mission events, keyed and deduplicated |

**Stages and objectives.** Bethesda and CD Projekt RED keep the designer's progress (stages, facts) apart from what the player sees (objectives, journal), joined by script calls or journal nodes. Hitman and Horizon tie each player-facing goal to one piece of state. For generated missions the second is safer, because nothing can drift.

**Roles.** Every system that reuses one definition across worlds binds names to entities when the mission starts: Bethesda's aliases [4], Daggerfall's resources [44], Paradox's scopes [11]. The common rules:
- an optional flag;
- failure to start when a required role cannot bind;
- binding once, not continuously.

Hitman points at fixed repository ids in a hand-made level [21]. That suits Forge's generator, which knows what it placed.

**Counters and progress.** Destiny's definition is the most complete model of progress semantics [24]: target, direction, over-completion, un-completion, a visibility threshold. CD Projekt RED's Objective Counter node [15] and Quest Machine's counters [36] cover the "3 of 5" case.

**Failure and cancellation are different outcomes.** Arma's tasks separate FAILED from CANCELED [41]. Verse cancels the losers of a race without failing them [32]. A Forge objective should show *missed* (cancelled) differently from *failed*.

**Level and edge.** Most conditions in this survey are states: in an area, holds an item, a fact's value, dead. Hitman's handlers and Story Manager starts are events. Two consequences:
- A state can be re-tested after a save, a hot reload, a late join or a rollback.
- An event that arrives before its step is active is lost unless something stores it. Peacock replays the current event after a timer-driven transition for that reason [22].

This argues for level-triggered conditions first (§7).

---

## 3. Concurrency, cancellation and ordering

| Pattern | Verse [32] | CD Projekt RED [15] | StateTree [27][28] | Naughty Dog [26] | Arma [40] | Proposed for Forge v1 |
|---|---|---|---|---|---|---|
| Sequence | block | node chain | child states | track | waypoint order | `Seq` |
| All, then continue | `sync` | *And* | *TasksCompletion: All* | — | AND waypoint | `Sync` |
| First wins, cancel the others | `race` | Xor plus Cut Control | transition on first completion (or *Any*) | `go` aborts blocks | OR waypoint | `Race` |
| First wins, others continue | `rush` | Xor | — | — | — | left out |
| Background, bound to a scope | `branch` | parallel path inside a phase | *Run Parallel Tree*, global tasks | concurrent tracks | — | `Branch` |
| Unbounded background | `spawn` | — | — | — | — | left out |

**Cancellation and cleanup:**
- Verse cancels from parent to child and runs `defer` blocks, which cannot suspend [32].
- Naughty Dog aborts a state's blocks on a transition and advises an exit block [26].
- CD Projekt RED kills the signals inside a phase when its output fires, and Cut Control kills a target node's signal [15].
- Bethesda removes alias data when the quest stops [5].
- Arma's scripted framework is reported to give a parent's final state to its unfinished children (community report) [41].

The common rule: a structure that ends stops everything under it, and each node may run cleanup that cannot wait.

**Ordering inside one tick:**
- Hitman (as reimplemented): timers first, then the event; handlers in listed order; the first transition wins [22].
- StateTree: transitions from the leaf upward, the first that succeeds wins, and two events in one frame can drop a parent's transition [27][29].
- Factorio: handlers in a fixed order of mod dependency, then name [37].

Forge needs a written total order, because the order is part of the digest.

**Petri nets.** The CD Projekt RED graph is a token net: phases are subnets, *And* joins, *Xor* takes the first arrival, Cut Control removes tokens. Researchers check such nets for reachability and soundness [48]. Forge's tree is a restricted, structured form: no arbitrary joins, every token scoped to a parent. It is easier to save, reconcile and validate, at the cost of some freedom.

---

## 4. Saves and hot reload

**What gets saved:**
- Bethesda: per-record deltas against the base records [9], including stage flags (shown for Oblivion), alias fills and script variables.
- CD Projekt RED: facts and the position of each signal; phases can forbid saving [15][17].
- Hitman: the state name and context, per scope (profile, contract, session) [21].
- Questify: a serialised array per quest, restored into fresh instances of the original resource [35].
- Factorio: `storage`, plus the list of migrations already run [37].

**A changed definition against an old save:**
- **CD Projekt RED** keeps a deletion marker where a node was removed, so a saved signal still has somewhere to go [15].
- **Factorio** runs migrations once per save and records them [37]. Version changes trigger `on_configuration_changed` [37].
- **Bethesda** has no general mechanism. Aliases fill only at start, so aliases added later stay empty on a running quest [7]. Mod authors ask players not to update start-game-enabled quests mid-game [8].
- **Horizon** could not revert story steps once taken [18].
- **D-035** already gives Forge schema versions, migrations, renames by alias and "removed names never reused" for records. Mission *state* needs the same at the step level.

**What makes hot reload safe:**
1. **State outside the definition.** Chop Chop's `_isDone` on the asset is the counter-example [34].
2. **Stable ids per step, never reused;** renames declared. CD Projekt RED's marker [15], D-035's aliases.
3. **Level-triggered conditions.** After a reload a step re-tests the world instead of waiting for an event that already passed.
4. **No suspended code in state.** A suspended coroutine cannot be saved, which is why Factorio removed its coroutine library (scripting.md) [37].
5. **One deterministic reconcile rule** that maps old state onto the new tree. It is the same rule for loading an old save (§7.8).
6. **In multiplayer,** a content change changes the manifest hash (D-035). Hot reload is then a development and single-player feature; a running server takes new content as a new package set.

---

## 5. Determinism and networking

**Who owns mission state:**
- Destiny hosts the mission simulation on a server and moves player state peer to peer [25].
- Verse code is server-authoritative, with client prediction hidden from it (forum post) [33].
- Arma has *server only* triggers [39] and tasks that can be set globally [41].
- Lyra's gameplay messages are local, not replicated [31].
- Factorio runs the same scenario code on every peer in lockstep [37][38].

**Forge's case (D-010)** is the lockstep case with prediction. The server and every client run the mission system inside the predicted step. A wrong prediction shows up as a digest mismatch, and the client goes back to the server's snapshot. So:
- **Mission state is simulation state.** It is digested, integer-only, in a canonical order (sorted maps), with no floats and no wall-clock time. Ticks, not seconds.
- **Role binding is deterministic.** Candidates are sorted by stable id. Any choice comes from `Seed::derive` of the mission id and the start tick (D-016).
- **The definition is the same everywhere.** Records come through the manifest hash (D-035). A generated mission is generated on every machine from the map's seed, as D-034's weather is, or shipped as a package. It is never a definition the server streams at run time (see §6, Zynga).
- **The order is total:**
  - the mission system runs at a fixed place in the system order;
  - missions are visited by runtime id;
  - steps are visited depth-first in declaration order;
  - actions apply in order;
  - events are emitted in that order (§7.7).
- **UI and audio under prediction** follow Photon Quantum [49]:
  - events raised on predicted ticks can be replayed or cancelled;
  - Quantum removes duplicates by hashing the event's data, id and tick, and calls the view to confirm or cancel each one;
  - *synced* events wait for the server's input: no false positives or negatives, at the cost of latency.
  - For Forge: objective text, markers and progress are drawn from the state (always right after a correction). One-off cues (a sound, a banner) come from events keyed by mission, step, kind and tick. Cheap cues play when predicted; heavy ones (mission complete) wait until the tick's digest is confirmed.
- **Debugging divergence:** Factorio's per-tick checksum and full-map mode [38] map onto D-010's digests. A mission digest per tick, logged beside the others, would locate a divergent step.

---

## 6. Patents

Searched on 2026-10-18 by web search. Google Patents answered HTTP 503 and Justia HTTP 403 throughout, so dates and claims come from Patent Arcade, KISTI, the EPO and search snippets. "Claim 1 read" means its elements were seen on Patent Arcade. Everything else is title, abstract or description only. Expiry dates assume 20 years from the US filing, without term adjustments, and are not verified.

| Patent | Holder | Dates, status | What it covers | Does the proposal touch it? |
|---|---|---|---|---|
| **US 9,463,386 B1** "State machine scripting in computer-implemented games" [50] | Zynga (original assignee) | provisional 2011-11-08; application 13/460,181 (2012); status not verified; about 2032 | *Claim 1 from a search snippet only:* a game engine module; a state machine module that gets a state machine definition **from a server**; the definition gives the states of **graphical objects** in the game's display; scripting functions with arguments fixed at run time decide states and transitions; objects drawn by state | **Probably not as proposed.** Forge's records ship in packages under the manifest hash, or are generated on each machine from the seed. They describe mission steps, not how objects are displayed. It comes closer if a server sends mission records to clients at run time, or if records set objects' visual states. **Read the full claims before either, and ask the owner.** Prior art predates 2011 (Naughty Dog's state scripts [26], Bethesda's quests, CD Projekt RED's quest graphs), but a granted patent is presumed valid |
| **US 9,821,230 B2** "Data-driven state machine for user interactive displays" [51] | Zynga | continuation of 13/460,123 (filed 2012-04-30; priority 2011-11-08); filed 2016-04-06; granted 2017-11-21; about 2032 | from snippets: dependent claims on a user interaction moving a graphical object between states, server-side and client-side state machines, and a token string the client parses to invoke a script; claim 1 not read | Same reading as above. Parsing tokens into script calls is far from records evaluated by native code |
| **US 9,463,376 B1** (published as US 2014/0364238; WO 2014/197887) "Dynamically generated in-game objectives" [52] | Kabam (applicant); current owner not checked | filed 2013-06-07; the US grant number comes from a family listing, grant date not seen; the PCT application lapsed; about 2033 if granted and maintained | published claims (snippet): objectives for an online game generated dynamically and at random, by selecting components such as **task type, task amount or time allotted**; a reward decided at random on completion | **Unclear without the granted claims.** A generator that picks objective types, counts and time limits for a map comes near the published wording. Before the generator picks them at random, read the granted claims and ask. Prior art: Daggerfall's generated quests with clocks (1996) [44], Arma 2's SecOps (2009) [42] |
| **US 9,174,128 B2** "Dynamic quests in game", with continuations US 9,731,207 and US 10,004,992 [53] | Zynga | priority 2012-04-26; granted 2015-11-03; about 2032 | **claim 1 read:** when a player has fewer quests than a threshold, pick a template, pick two or more of its tasks without user input, work out the in-game currency needed to buy their completion, choose rewards from that amount, give the quest; reward after all tasks | **No.** Forge prices nothing in currency and does not top up a player's quest count. The continuations' claims were not read |
| **US 11,338,202 B1** "Methods and systems for dynamic quest diversification"; **US 11,612,822** "…dynamic quest generation"; **US 11,529,563** "…co-op gaming using dynamic quests" [54] | Sony Interactive Entertainment (confirmed for '202; the others share inventors and text) | '202 filed 2020-11-30, granted 2022-05-24; about 2040 | descriptions: quests generated from a player's history (aversion to or appetite for quest types), limited in complexity or duration; co-op spaces entered by player metrics; claims not read | **No,** while missions come from the map and not from a player's history. Read before any personalisation |
| **US 11,645,560** "Processing in-content challenges for online gaming systems" [55] | Versus LLC | application 17/198,187, filed 2021-03-10 | **claim 1 read:** challenges with criteria; on game launch a **machine-learning model** predicts which challenges a player will likely meet; per-challenge scores updated from event data until met; content generated and sent | **No.** No learned prediction, no external challenge platform |
| **US 10,926,179** "Nemesis characters, nemesis forts, social vendettas and followers…" [56] | Warner Bros. Entertainment | granted 2021-02-23; to about 2035 | managing NPCs and power centres through character hierarchies | **No for v1.** Read before missions come from NPCs' memory or hierarchies |
| **US 10,086,276** "Procedural game content generation via interactive non-player game entities" [57] | Disney Enterprises | filed 2015-12-03; granted 2018-10-02; about 2035 | title and listing only | **No for v1.** Read before NPCs generate missions at run time |

**Also seen, not relevant to this proposal:**
- **US 7,637,806** (Rampart Studios, granted 2009). Random goals given to NPCs per in-game time frame, with steps for players to help. Filing date not seen; likely expired or close to it [58].
- **US 9,339,731** (Intellectual Discovery). On connection, the server sets chained, one-time and repeatable quests from user information [59].
- **US 10,076,707** (Capcom). Quest clues that change with the number of completions [60].
- **US 10,272,341** (Amazon). Generated levels scored by a learned model; expired for unpaid fees, per Google's listing [61].
- **US 9,367,543** (Microsoft, "Game achievements system", priority 2004). Likely expired; not checked [62].
- **US 11,245,776** (a platform data model for activities and events; EP counterpart priority 2018-10-22). It concerns a console platform's data service, not a game's records [63].
- Zynga's **US 8,449,397** family ("Secure, parallel, and independent script execution") is covered in scripting.md §4. Missions here are records read by engine code, not scripts. The same caution applies if records ever grow into a language.

**Nothing was found** on:
- quest graphs as such;
- stage and objective tracking in single-player engines;
- role or alias binding;
- structured-concurrency operators in content data;
- reconciling saved state with changed definitions.

The search was throttled, so this is not exhaustive.

---

## 7. Proposed schema for Forge's first version (proposal)

**This is a proposal for the owner and for #287, not a decision.** RON-like pseudo-records follow D-035's conventions: `package:path` ids, RON structs and enums, and unknown fields rejected.

### 7.1 Principles

1. **One tree of steps** per mission, combined by `Seq`, `Sync`, `Race` and `Branch` with Verse's meanings (§3).
2. **Leaf steps wait on level-triggered conditions,** states the world already holds, rather than one-off events (§2). A completed step latches.
3. **Roles are the only references to the world.** They are bound once when the mission starts, saved as stable ids, and checked at load.
4. **Objectives are a view.** Each is linked from steps; its state and progress are derived, never stored.
5. **State is a sparse map of step id to step state, plus integer vars.** It is digested and saved. "Saved by stage" (D-072) means this map. It generalises Bethesda's stage numbers, because a tree with `Sync` has no single stage.
6. **One reconcile rule** serves hot reload and old saves (§7.8).
7. **Effects are few.** Rewards and consequences go out as events that game code handles. Anything more waits for Luau commands (D-072) or a later record version.

### 7.2 The record

```ron
// demo/missions/village_cellar_key.ron (proposal, not an implemented format)
Mission(
    id: "demo:village/cellar_key",
    revision: 3,                         // authors bump it; saves also keep the record's content hash
    title: "demo:mission.cellar_key.title",          // localisation key
    roles: {
        "players": (bind: Players),                                   // v1: one shared state for all players
        "key":     (bind: Placed("house_03/cellar/key")),             // stable names written by the map or generator
        "cellar":  (bind: Placed("house_03/cellar/volume")),
        "gate":    (bind: Placed("village/gate/volume")),
        "river":   (bind: Placed("village/river_bank/volume")),
        "guards":  (bind: Tagged(tag: "demo:npc/guard", within: Some("village"), pick: All)),
        "elder":   (bind: Tagged(tag: "demo:npc/elder", within: None, pick: Seeded(1)), optional: true),
    },
    vars: { "alarms": 0 },               // i64 counters and flags
    objectives: {
        "find_key": (text: "demo:obj.find_key", markers: ["cellar"]),
        "leave":    (text: "demo:obj.leave",    markers: ["gate", "river"]),
        "guards":   (text: "demo:obj.guards",   optional: true),                 // shows "n / 3"
        "quiet":    (text: "demo:obj.quiet",    optional: true, hidden: true),
    },
    flow: Sync(id: "root", steps: [
        Seq(id: "main", steps: [
            Branch(id: "alarm_watch", step:
                Wait(id: "too_loud", until: Var("alarms", AtLeast(3)),
                     on_success: [EndMission(Failed)])),
            Wait(id: "got_key", until: Holds("players", Role("key"), 1),
                 fail_if: Some(After(seconds: 600)), objective: Some("find_key")),
            Race(id: "escape", objective: Some("leave"), steps: [
                Wait(id: "by_gate",  until: Inside("players", "gate")),
                Wait(id: "by_river", until: Inside("players", "river")),
            ]),
        ]),
        Wait(id: "guards_down", optional: true, objective: Some("guards"),
             until: CountOf(Dead("guards"), at_least: 3)),
        Wait(id: "stay_quiet", optional: true, objective: Some("quiet"),
             until: Never, fail_if: Some(Var("alarms", AtLeast(1)))),   // cancelled ("missed") when root ends: shown as kept
    ]),
    on_success: [Emit("demo:mission/cellar_key/done"), Reward("demo:rewards/cellar_key")],
    on_failure: [Emit("demo:mission/cellar_key/failed")],
    renamed: { "take_key": "got_key" },  // old step id -> new: reconcile follows it (§7.8)
)
```

### 7.3 Steps

Every step has an `id`, unique in the mission and never reused (a tombstone in `renamed` if moved). Every step may also have `objective`, `optional`, `on_success`, `on_failure` and `on_cancel` (actions).

| Step | Starts | Succeeds | Fails | When it ends |
|---|---|---|---|---|
| `Seq(steps)` | its first child | when its last child succeeds | when a required child fails | cancels its running branches |
| `Sync(steps)` | all children, in order | when every required child has succeeded | when a required child fails | cancels every child still running, optional ones included |
| `Race(steps)` | all children, in order | if the first child to end succeeded | if the first child to end failed | cancels the others; ties go to declaration order |
| `Branch(step)` | its child, then succeeds at once | immediately | never | the child runs until the enclosing composite ends, then is cancelled; its outcome matters only through its own actions (Verse's `branch`) |
| `Wait(until, fail_if)` | — | when `until` holds | when `fail_if` holds (tested first) | — |
| `Do(actions)` | runs its actions | the same tick | never | — |

- **Optional steps:** a failure is not passed to the parent, and a parent does not wait for them.
- **Cancelled** is a third outcome. It runs `on_cancel`, and an objective shows it as *missed*.
- **The mission** succeeds or fails with its root, or with `EndMission`. Ending cancels everything still running, in tree order.

### 7.4 Conditions (v1), all level-triggered

| Condition | Holds when | Covers | Precedents |
|---|---|---|---|
| `Inside(who, volume)` | an entity of role `who` is inside the volume entity of role `volume` | a volume entered | Arma's area triggers [39]; Cyberpunk's Pause on the player in an area [17] |
| `Near(who, target, metres)` | the distance is at most the radius (fixed point after load) | an entity reached | Horizon's verbs [18]; *goto* [45] |
| `Holds(who, Role(r) \| Kind(item), n)` | an inventory holds at least n | an item picked up | REDkit's item quantity [16]; Chop Chop's CheckItem [34] |
| `Dead(role)`, `Killed(role, by: role)` | all entities of the role are dead (by a member of `by`) | an entity killed | Hitman's *Kill* with a repository id [21]; REDkit's *killed* [16] |
| `After(seconds)` | the step has run that long (converted to ticks at load; fractions rejected) | a timer | Hitman's `$timer` [22]; Daggerfall's Clock [44]; Arma's timeout [39] |
| `Var(name, cmp)` | an integer var compares | flags, counters | Cyberpunk's facts [17]; Mass Effect's plot variables [43] |
| `CountOf(cond_on_set, at_least: n)` | at least n members of a set role satisfy | "3 of 5", progress | Destiny's `completionValue` [24]; REDkit's Objective Counter [15] |
| `AllOf([..])`, `AnyOf([..])`, `Not(..)`, `Never` | combinators | all-of, any-of | Paradox [10]; Questify [35] |

What each condition needs from the engine:
- overlap sets for volumes;
- positions;
- inventories;
- death records that keep the killer's stable id.

Those systems must run before the mission system in the tick.

### 7.5 Actions (v1)

| Action | Effect |
|---|---|
| `Set(var, v)`, `Add(var, d)` | change an integer var; visible to steps later in the same pass |
| `Emit(tag)` | a mission event with a namespaced, hierarchical tag (as Gameplay Tags [30]) for UI, audio and game code |
| `Reward(record_id)` | a reward event that the game crate applies; no economy in the mission system |
| `EndMission(Succeeded \| Failed)` | ends the mission at once |

### 7.6 State

```ron
MissionState(
    mission: "demo:village/cellar_key",
    record_hash: "blake3:…",            // the definition this state was written against
    revision: 3,
    status: Running,                     // Running | Succeeded | Failed | Cancelled
    started: 1200,                       // tick
    roles: { "key": ["demo:village/house_03/cellar/key"], "guards": ["…/guard_1", "…/guard_2", "…/guard_3"] },
    vars: { "alarms": 1 },
    steps: {                             // only steps that have started; ids interned to u32 at run time
        "root": Running(since: 1200), "main": Running(since: 1200),
        "alarm_watch": Succeeded(at: 1200), "too_loud": Running(since: 1200),
        "got_key": Succeeded(at: 4410), "escape": Running(since: 4410),
        "by_gate": Running(since: 4410), "by_river": Running(since: 4410),
        "guards_down": Running(since: 1200), "stay_quiet": Failed(at: 3021),
    },
)
```

Objective states and progress are derived from this map and the world, and never stored.

### 7.7 Evaluation in one tick (the total order)

1. The mission system runs once per tick, at a fixed place after movement, combat, inventory and overlaps.
2. Missions are visited by runtime id (from sorted string ids, D-035).
3. In each mission, one depth-first pass in declaration order:
   - each running `Wait` tests `fail_if`, then `until`;
   - an ended step tells its parent at once, so a `Seq` starts its next child, which this pass reaches later;
   - a `Race` cancels the later siblings;
   - a step starts at most once per tick and v1 has no loops, so a pass is bounded by the tree's size.
4. Actions apply in order. Events are appended to the tick's event list in that order.
5. The state goes into the tick's digest.

### 7.8 Saves, loads and hot reload: the reconcile rule

Run when a record changes in development, and when a save written against another `record_hash` is loaded:
1. Map old step ids through `renamed`.
2. Keep the state of steps that still exist with the same kind. Drop the rest.
3. Repair from the root down:
   - a running `Seq` runs its first unfinished child;
   - a running `Sync` or `Race` runs all its unfinished children;
   - a `Branch` child runs while its scope does;
   - steps started here get `since = now`.
4. Vars: keep by name; add new ones with their defaults; drop removed ones.
5. Roles: keep those already bound; bind new ones now. A required role that cannot bind is an error in development; in a release, see question 7.
6. Store the new hash, and log what changed.

This is what CD Projekt RED's deletion marker and Factorio's migrations do by hand [15][37]. Explicit migration actions can be added later if a case needs them.

### 7.9 Networking and events

- **Same evaluator everywhere.** It runs on the server and on predicting clients.
- **Generated missions** are generated on every machine from the map seed (preferred), or shipped as a package.
- **UI** reads state. One-off cues come from events keyed by mission, step, kind and tick. Heavy cues wait for the confirmed digest (§5).

### 7.10 What the generator writes, and checks

- **Output.** A `Mission` record, plus placements with stable names that its roles bind by `Placed`. `Tagged` stays for hand-made maps.
- **Vocabulary.** Doran and Parberry's actions map onto v1 [45]:
  - *goto* → `Inside` or `Near`;
  - *get* → `Holds`;
  - *kill* → `Dead` or `Killed`;
  - *defend* for a time → `After` with `fail_if`.
  - *talk*, *give*, *use*, *escort* and *report* wait for later conditions.
- **Validation,** at generation and in a test:
  - every role binds;
  - every `Inside` or `Near` target is reachable from the spawn with the navigator's capsule (the house rule);
  - no `Holds` item sits behind a lock whose key comes later. Keep the key–lock graph, as Unexplored does [46];
  - every timer is at least the path time times a margin;
  - every `Race` arm can end;
  - the record's digest is logged with the seed.

### 7.11 Left out for now

- `rush` and `spawn`; loops and repeats; runtime random choices (choose at generation time instead).
- Edge-triggered events as conditions ("entered again", "shot fired").
- Talk, give, use, escort; dialogue (ink later).
- Spawning, enabling or disabling entities; AI orders. Those come later through records or Luau commands (D-072).
- Scoped effects that undo themselves on exit (Bethesda's alias data [5]). Markers shown only while an objective is active come free from §7.1.4.
- Missions started by world events (Story Manager [6], on_actions [10]). In v1 a map lists the missions it starts.
- Per-player progress in co-op, and profile-scope state (Hitman's scopes [21]).
- Checkpoint restarts.
- Floats in state.
- Personalisation from player history (§6).

### 7.12 Open questions for the owner

1. **Level-triggered only in v1,** with no event conditions?
2. **Ties:** when `fail_if` and `until` hold in the same tick, failure wins (proposed)?
3. **Race:** does the first arm to *end* decide the race even if it failed (Verse's rule, proposed), or only the first to succeed?
4. **Optional children of a `Sync`:** cancelled ("missed") when the required ones finish (proposed), or does the `Sync` wait for them?
5. **Co-op:** `Inside("players", …)` holds when *any* player is inside (proposed). Is a per-player form needed later?
6. **Cadence:** evaluate every tick (proposed; measure it), or every N ticks as Arma and Questify do (about 0.5 s)?
7. **A required role that cannot bind** when an old save is loaded in a release: fail the mission, or skip the steps that use it?
8. **Generated records:** generated on every machine from the seed (proposed), or shipped as a package?
9. **Patents:** read the Zynga state-machine claims (US 9,463,386, US 9,821,230) before records are ever sent by a server at run time? Read Kabam's US 9,463,376 claims before the generator picks objective types, counts or time limits at random?

---

## Sources

1. Creation Kit wiki, "Quest Stages Tab" — https://ck.uesp.net/wiki/Quest_Stages_Tab (also https://cs.uesp.net/wiki/Quest_Stages_Tab, https://geck.uesp.net/wiki/Quest_Stages_Tab)
2. Fallout 4 Creation Kit wiki, "Quest" — https://falloutck.uesp.net/wiki/Quest ; tutorial https://ck.uesp.net/wiki/Fallout_4_Simple_Fetch_Quest_Tutorial_Chapter_8:_Quest_Setup
3. Creation Kit wiki, "Quest Objectives Tab" — https://ck.uesp.net/wiki/Quest_Objectives_Tab ; "SetObjectiveDisplayed" — https://falloutck.uesp.net/wiki/SetObjectiveDisplayed_-_Quest
4. Creation Kit wiki, "Quest Alias Tab" — https://ck.uesp.net/wiki/Alias
5. Creation Kit wiki, "Bethesda Tutorial Quest Aliases" — https://ck.uesp.net/wiki/Bethesda_Tutorial_Quest_Aliases
6. Creation Kit wiki, "Bethesda Tutorial Story Manager" — https://skyrimck.uesp.net/wiki/Bethesda_Tutorial_Story_Manager ; "Radiant Story" — https://ck.uesp.net/wiki/Radiant_Story
7. Beyond Skyrim wiki, "Implementation Troubleshooting Reference" — https://wiki.beyondskyrim.org/wiki/Arcane_University:Implementation_Troubleshooting_Reference ; Nexus forum — https://forums.nexusmods.com/topic/3339845-quest-aliases-do-not-fill-on-conditions
8. Nexus Mods page warning against updating on a running save — https://www.nexusmods.com/skyrimspecialedition/mods/22395 ; forum — https://forums.nexusmods.com/topic/940526-question-on-scripts-attached-to-sge-quests
9. UESP, "Skyrim Mod:Save File Format" — https://en.uesp.net/wiki/Tes5Mod:Save_File_Format ; Oblivion's quest change record — https://content2.uesp.net/wiki/Tes4Mod:Save_File_Format/QUST
10. CK3 wiki, "Event modding" — https://ck3.paradoxwikis.com/Event_modding
11. CK3 wiki, "Scopes" — https://ck3.paradoxwikis.com/Scopes
12. CK3 wiki, "Story cycles modding" — https://ck3.paradoxwikis.com/Story_cycles_modding
13. Stellaris wiki, "Event modding" — https://stellaris.paradoxwikis.com/Event_modding
14. EU4 wiki, "Mission modding" — https://eu4.paradoxwikis.com/Mission_modding
15. CD Projekt RED, REDkit wiki, "HOW-TO: Use quest nodes" — https://cdprojektred.atlassian.net/wiki/spaces/W3REDkit/pages/6327742/HOW-TO%3A+Use+quest+nodes
16. CD Projekt RED, REDkit wiki, "HOW-TO: Create a new quest" — https://cdprojektred.atlassian.net/wiki/spaces/W3REDkit/pages/6327243 ; "HOW-TO: Use quest conditions and functions" — https://cdprojektred.atlassian.net/wiki/spaces/W3REDkit/pages/135987240
17. REDmodding wiki, "Quests: facts and files" — https://wiki.redmodding.org/cyberpunk-2077-modding/for-mod-creators-theory/files-and-what-they-do/file-formats/quests-.scene-files/quests-facts-and-files
18. L. Szczepanski, "Building Non-Linear Narratives in Horizon Zero Dawn", GDC 2017 — https://www.guerrilla-games.com/media/News/Files/GDC2017_Szczepanski_BuildingNonLinearNarrativesInHorizonZeroDawn.pdf (read through https://www.slideshare.net/slideshow/building-nonlinear-narratives-in-horizon-zero-dawn/72990463)
19. Massive at GDC 2014 on Snowdrop — https://mcvuk.com/?p=77365 ; Ubisoft News, "The Division 2: how the Snowdrop engine…" — https://news.ubisoft.com/en-us/article/3tnmRuL2hiwTdCHxXJlbiD
20. J. Horneman, "Missions Played As…", GDC 2021 AI Summit (abstract) — https://www.gdcvault.com/play/1027145/AI-Summit-Missions-Played-As
21. The Peacock Project: contract data and types — https://github.com/thepeacockproject/Peacock/blob/master/contractdata/DUBAI/ONTOPOFTHEWORLD.json ; https://github.com/thepeacockproject/Peacock/blob/master/components/types/types.ts
22. The Peacock Project, StateMachineParser (`src/handleEvent.ts`, `src/index.ts`) — https://github.com/thepeacockproject/StateMachineParser
23. D. Isla, "Building a Better Battle: Halo 3 AI Objectives", GDC 2008 (abstract) — https://www.halo.bungie.org/bwu/index.html?item=164 ; https://bnetarchive.elaztek.com/Inside/publications.html
24. Bungie.net API, `DestinyObjectiveDefinition` — https://bungie-net.github.io/multi/schema_Destiny-Definitions-DestinyObjectiveDefinition.html
25. J. Truman, "Shared World Shooter: Destiny's Networked Mission Architecture", GDC 2015 — https://www.gdcvault.com/play/1022247/Shared-World-Shooter-Destiny-s (see netcode.md)
26. J. Gregory, "State-Based Scripting in Uncharted 2: Among Thieves", GDC 2009 — https://www.slideshare.net/slideshow/statebased-scripting-in-uncharted-2-among-thieves/7093984 ; Naughty Dog, GDC 2008 presentations — https://www.naughtydog.com/blog/presentations_from_game_developers_conference_2008
27. Epic Games, "Overview of StateTree" — https://dev.epicgames.com/documentation/en-us/unreal-engine/overview-of-state-tree-in-unreal-engine
28. Epic Games API: `StateTreeState` (TasksCompletion, linked asset) — https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/StateTreeState ; `FStateTreeRunParallelStateTreeTask` — https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/StateTreeModule/FStateTreeRunParallelStateTreeTa- ; `FStateTreeEvent` — https://dev.epicgames.com/documentation/unreal-engine/API/Plugins/StateTreeModule/FStateTreeEvent ; `StateTreeEventDesc` — https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/StateTreeEventDesc
29. Unreal forums, "State Tree Event Transition order" — https://forums.unrealengine.com/t/state-tree-event-transition-order/2737753
30. Epic Games, "Using Gameplay Tags to Label and Organize Your Content" (2017) — https://www.unrealengine.com/en-US/tech-blog/using-gameplay-tags-to-label-and-organize-your-content-in-ue4 ; `FGameplayTag` — https://docs.unrealengine.com/4.27/en-US/API/Runtime/GameplayTags/FGameplayTag/
31. Unreal Community Wiki, "GameplayMessageSystem" — https://unrealcommunity.wiki/gameplaymessagesystem-74b916
32. *The Book of Verse*, "Concurrency" — https://github.com/verselang/book/blob/main/docs/14_concurrency.md ; Epic, "Time flow and concurrency in Verse" — https://dev.epicgames.com/documentation/en-us/uefn/time-flow-and-concurrency-in-verse ; `CExprBranch` — https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/VerseCompiler/CExprBranch
33. Unreal forums, "Predicts" — https://forums.unrealengine.com/t/predicts/2762608
34. Unity Technologies, Open Project 1, `StepSO.cs` — https://github.com/UnityTechnologies/open-project-1/blob/main/UOP1_Project/Assets/Scripts/Quests/ScriptableObjects/StepSO.cs
35. Questify for Godot 4 — https://github.com/TheWalruzz/godot-questify
36. Pixel Crushers, "Quest Machine" release thread — https://discussions.unity.com/t/released-quest-machine-hand-written-and-procedurally-generated-quests/677637
37. Factorio Lua API, "Data Lifecycle" — https://lua-api.factorio.com/latest/auxiliary/data-lifecycle.html
38. Factorio Friday Facts #47, #55, #188 — https://factorio.com/blog/post/fff-47 ; https://factorio.com/blog/post/fff-55 ; https://factorio.com/blog/post/fff-188
39. BI wiki, "Eden Editor: Trigger" — https://community.bistudio.com/wiki/Eden_Editor:_Trigger ; `triggerInterval` — https://community.bistudio.com/wiki/triggerInterval ; `triggerTimeoutCurrent` — https://community.bistudio.com/wiki/triggerTimeoutCurrent
40. BI Feedback Tracker T76188 (AND/OR waypoints) — https://feedback.bistudio.com/T76188 ; T185363 (non-repeatable triggers) — https://feedback.bistudio.com/T185363
41. BI wiki, "Arma 3 Task Framework" — https://community.bistudio.com/wiki/Arma_3_Task_Framework ; `BIS_fnc_taskCreate` — https://community.bistudio.com/wiki/BIS_fnc_taskCreate
42. BI wiki, "Secondary Operations Manager" — https://community.bistudio.com/wiki/Secondary_Operations_Manager ; "MP Combat Patrol" — https://community.bistudio.com/wiki/MP_Combat_Patrol
43. ME3Tweaks, "Introduction to Plot Management" — https://github.com/ME3Tweaks/ME3Explorer/wiki/Introduction-to-Plot-Management
44. Daggerfall Workshop, quest source format — https://www.dfworkshop.net/?p=2705 ; forum, "Chaining Quest Timers" — https://forums.dfworkshop.net/viewtopic.php?p=8844 ; UESP, "Quest hacking guide" — https://en.uesp.net/wiki/Daggerfall:Quest_hacking_guide
45. J. Doran, I. Parberry, "A Prototype Quest Generator Based on a Structural Analysis of Quests from Four MMORPGs", PCG 2011 — https://ianparberry.com/pubs/pcg2011.pdf ; project page — https://ianparberry.com/research/quests ; third-party reimplementation — https://gist.github.com/Ovid/0674ef8fe906f10cc8f2
46. J. Dormans, "Adventures in Level Design", PCG 2010 — https://www.pcgworkshop.com/archive/dormans2010adventures.pdf ; Boris the Brave, "Dungeon Generation in Unexplored" — https://www.boristhebrave.com/2021/04/10/dungeon-generation-in-unexplored/
47. R. van Rozen, J. Dormans, "Adapting Game Mechanics with Micro-Machinations", FDG 2014 — https://ir.cwi.nl/pub/23138
48. Petri nets for games and quests: Araújo and Roque, DiGRA 2009 — https://dl.digra.org/index.php/dl/article/download/493/493 ; Vega, Grünvogel and Natkin (2004) — https://sgruenvo.web.th-koeln.de/?p=255 ; Oliveira (2012, in Portuguese) — https://repositorio.ufu.br/handle/123456789/12536
49. Photon Quantum, "Game Events" — https://doc.photonengine.com/quantum/current/manual/quantum-ecs/game-events
50. US 9,463,386 B1 — https://patents.google.com/patent/US9463386 ; https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/9463386
51. US 9,821,230 B2 — https://patents.google.com/patent/US9821230 ; https://scienceon.kisti.re.kr/srch/selectPORSrchPatent.do?cn=USP2017119821230 ; US 2016/0239184 — https://patents.justia.com/patent/20160239184
52. US 2014/0364238 — https://patents.justia.com/patent/20140364238 ; WO 2014/197887 — https://patents.google.com/patent/WO2014197887A1/en
53. US 9,174,128 — https://www.patentarcade.com/video-game-patent/US9174128/dynamic-quests-in-game ; https://patents.google.com/patent/US9174128 ; US 9,731,207 — https://patents.google.com/patent/US9731207B2/en
54. US 11,338,202 — https://patents.google.com/patent/US11338202 ; US 11,612,822 — https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/11612822 ; US 11,529,563 — https://patents.justia.com/patent/11529563
55. US 11,645,560 — https://www.patentarcade.com/video-game-patent/US11645560/processing-in-content-challenges-for-online-gaming-systems
56. US 10,926,179 — https://www.patentarcade.com/video-game-patent/US10926179/nemesis-characters-nemesis-forts-social-vendettas-and-followers-in-computer-games
57. US 10,086,276 — https://patents.google.com/patent/US10086276 ; https://patents.justia.com/patent/20170157510
58. US 7,637,806 — https://www.patentarcade.com/?p=524
59. US 9,339,731 — https://patents.google.com/patent/US9339731 ; https://patents.justia.com/patent/9339731
60. US 10,076,707 — https://www.patentarcade.com/video-game-patent/US10076707/video-game-with-dynamic-quest-clues-based-on-number-of-completions
61. US 10,272,341 — https://patents.google.com/patent/US10272341
62. US 9,367,543 — https://www.patentarcade.com/video-game-patent/US9367543/game-achievements-system
63. US 11,245,776 — https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/11245776 ; EP 3870322 — https://data.epo.org/gpi/EP3870322A4

## Checked and not confirmed

- **UESP's Creation Kit wikis** (ck., falloutck., skyrimck.uesp.net) and **the BI wiki** (community.bistudio.com) refused automated fetches (HTTP 403). Their content here comes from search-engine summaries of those pages. Worth reading in a browser:
  - the Quest Alias Tab's exact list of fill types;
  - Arma's default trigger interval (the "about 0.5 s" is a user note).
- **Google Patents** answered 503 and **Justia** 403 all day. Only two claim 1s were read (US 9,174,128 and US 11,645,560), plus a snippet of US 9,463,386's claim 1. Every expiry is an estimate.
- **PDFs could not be turned into text here:**
  - Horizon's deck: read through SlideShare's text;
  - Doran and Parberry: through the project page and a third-party reimplementation;
  - Dormans 2010: through Boris the Brave's article;
  - Halo 3 objectives: only the abstract.
- **Hitman:** the objective format is IOI's data as mirrored by Peacock. The evaluation order comes from Peacock's reimplementation, not from IOI.
- **Epic's Verse and Gameplay Message pages** render empty to the fetcher. Verse's semantics come from *The Book of Verse*, and Verse's server authority from a forum post.
- **Not found:**
  - Ubisoft's and Massive's mission data models;
  - Skyrim's quest change-form layout;
  - an official UE5 page on StateTree "On Event" transitions.

