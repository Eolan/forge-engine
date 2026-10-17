# The developer console and the settings (every demo, D-069)

Run any demo, then press the key under Esc (² on an AZERTY keyboard, ` on QWERTY). Decision:
[D-069](../DECISIONS.md) (#278). Research: [research/dev-console.md](../research/dev-console.md).

![The physics lab with the console open over the full profile](images/console-lab.png)

*The physics lab's dominoes with `--set app.console=1 --set r.display_info=2 --exec help
--exec r.display_info --exec "find display" --exec "r.display_info 9"`: the console's help,
a setting shown, found by a word, and a value out of range refused in red.*

## Settings

A setting is a console variable with a dotted name by system (`r.` rendering, `app.` the shell),
a type, a default, a range and a line of help. The engine and each demo register theirs; the
systems read them every frame, so a change applies at once.

A value comes from, each over the one before:
1. its default;
2. `forge.cfg` in the working directory (or `--cfg PATH`, or `FORGE_CFG`): one `name value` a
   line, `#` or `//` comments, edited by hand;
3. `--set NAME=VALUE` on the command line (repeatable);
4. the console, for the run only: nothing typed there is saved.

Names are lower case, in any case typed. A name that no setting has is reported once the demo is
ready (`r.typo (command line): no such setting`), in the log and the console; a scripted run (a
frame limit) fails on a misspelled `--set` rather than capture the wrong thing. Settings read at
start (marked "at start") take `forge.cfg` or `--set`, not the console. `--exec LINE`
(repeatable) runs console lines once the demo is ready, as if typed: with `--set app.console=1`,
a scripted capture shows the console, as above.

`forge.cfg` is the owner's own file: git ignores it, and the scripts (`tools/verify.sh`,
`captures.sh`, `map-check.sh`, `timings.sh` and the others that run demos) export
`FORGE_CFG=none`, so it never changes a check or an accepted image. `--no-cfg` reads no file
either.

## The console

- `NAME VALUE` (or `NAME=VALUE`) sets a setting; `NAME` alone shows it, with its range, its
  default and where its value came from.
- **Tab** completes a name as far as the names that fit agree, then a choice's value; once that
  cannot grow, each Tab steps to the next name that fits (**Shift+Tab** back), as Doom 3 and
  Readline do. The names that fit show under the line as it is typed, up to ten, with their
  values and help; the one Tab stands on is marked. When no name starts with what is typed, the
  names with a dotted part starting so come first (`probe` finds `gi.probe_spacing`), then those
  holding it anywhere.
- **Up** and **Down** recall the lines typed before. The history is kept between interactive
  runs (`%LOCALAPPDATA%/forge/console_history.txt`); the values are not.
- **PageUp** and **PageDown** scroll the log; **Ctrl+L** empties it; **Ctrl+U** the line.
- **Esc** or the key under Esc closes it. While it is open, the demo gets no keys.

The console's own commands: `help [NAME]` (or `NAME ?`), `find WORD` (names and help holding the
word), `list [PREFIX]`, `changed` (the settings not at their default, and where their values
came from), `reset NAME`, `toggle NAME` (flips an on/off setting, steps a choice or a small
number), `version`, `clear`. A change says what the value was: `r.display_info = 2 (was 1)`.

## What is registered so far

The shell's, in every demo:

| Name | What | Values | In a shipped game |
|---|---|---|---|
| `r.display_info` | the profile overlay (F1 steps through it) | 0 off, 1 on, 2 on (verbose) | yes |
| `app.console` | the console shown | 0, 1 | yes |
| `app.capture [PATH]` | command: saves the next frame as a PNG | | no |
| `app.quit` | command: closes the window | | yes |

The renderer's (`forge_render::settings`: one name, range and help whichever demo registers
them). The galleries (city-blocks, the island, the physics lab, the city plan) have them all; the
key that sets each is in its help. A flag the demo still takes (`--no-ao`, `--tonemap aces`)
sets the same setting, shown as "from command line": `--no-ao` and `--set r.ao=0` give the same
image to the pixel.

| Name | What | Values | Gallery default |
|---|---|---|---|
| `r.aa` | anti-aliasing (T steps through DLAA, TAA sharpened, TAA, off) | off, taa, dlaa | dlaa where it runs, else taa |
| `r.taa.sharpen`, `r.taa.sharpen_stops` | RCAS over TAA's image, and how strong | 0, 1; 0..8 stops | 1; 0.5 |
| `r.tonemap` | the tone curve (G) | agx, agx-punchy, aces, neutral, aces2, aces2-analytic | agx-punchy |
| `r.bloom`, `r.bloom.strength` | bloom (B), its share of the image | 0, 1; 0..1 | 1; 0.04 |
| `r.exposure.local` | local exposure, D-067 (X) | off, on, view | off |
| `r.ao`, `r.ao.radius` | ambient occlusion (N), its reach | 0, 1; 0.05..16 m | 1; 1.5 m |
| `r.probes` | the probes' diffuse light (P) | 0, 1 | 1 |
| `r.sky_light` | the sky's light (I) | 0, 1 | 1 |
| `r.shadows` | the sun's ray-traced shadows (J) | 0, 1 | 1 |
| `r.reflections.sky`, `r.reflections.rays` | the sky's reflection (F), mirror rays (Y) | 0, 1 | 1, 1 |
| `r.sun.disc` | the sun's disc at its true size (Z) | 0, 1 | 1 |
| `r.cull.occlusion` | occlusion culling (O) | 0, 1 | 1 |
| `r.lod`, `r.lod.error` | level of detail (L), its error in pixels ([ and ]) | 0, 1; 0.125..16 | 1; 1 |
| `r.sw_raster` | the software rasteriser (R) | off, auto, on | auto |
| `r.show.*` | debug views: `meshlets` (M), `lod` (K), `raster` (H), `ao` (V), `gi` (U), `culled`, `regions`, `wireframe` (Tab) | 0, 1 | 0 |

The asteroids, the planet and the meshlets bench take the same names for what they have, with
their own defaults where their scenes want another look, and add:

| Demo | Its own defaults | Added |
|---|---|---|
| asteroids | `r.tonemap aces` (its toe keeps space black), `r.sun.disc 0` (hard shadows), `r.ao.radius 2` | `r.dlss.mode` (U steps TAA, DLAA, then DLSS's modes), `r.exposure.compensation` (- and =), `r.translucency` (Y), `r.cull.cone` (C), `asteroids.dust` (V), `asteroids.paused` (P) |
| planet | `r.aa taa` (no DLAA yet) | `r.exposure.compensation`, `r.cull.cone`, `planet.paused` (P) |
| meshlets | `r.show.meshlets 1` | `r.cull.cone`, `r.cull.frustum` (V); F (freeze the culling camera) stays a key |

**The keys still differ between demos**, and the settings make it plain:

| Key | Galleries | Asteroids | Planet | Meshlets |
|---|---|---|---|---|
| P | `r.probes` | `asteroids.paused` | `planet.paused` | |
| X | `r.exposure.local` | `r.show.culled` | `r.show.culled` | |
| V | `r.show.ao` | `asteroids.dust` | | `r.cull.frustum` |
| Y | `r.reflections.rays` | `r.translucency` | | |
| U | `r.show.gi` | DLSS's modes | | |
| F | `r.reflections.sky` | | | freeze |

T, G, B, N, Z, J, O, L, K, M, H, R, Tab, `[` and `]` set the same setting wherever they are. One
key map for every demo is the owner's call (#278).

## Shipped games

A game built with forge-app's `shipping` feature keeps the console but only the settings marked
for release: they show information and give no advantage (the frame times and statistics, as
Star Citizen's `r_displayInfo`; the build's version in the console's header). Every other
setting keeps its default and is neither listed nor set, from the console, `forge.cfg` or the
command line: its name answers as unknown, so the build does not even show which names exist.
The research (`dev-console.md` §8) adds what a shipped game needs beyond this: the code that
would help a player (free camera, debug views, captures) compiled out, and "information" kept to
the machine, the build and the session, never the world (positions, hidden objects). The version reads `forge 0.1.0 (COMMIT, release)` when the build is given
`FORGE_BUILD_COMMIT`, else `dev build`.
