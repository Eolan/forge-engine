# Audio — the mixer, rendered headless (`forge-audio`)

Phase 6's first step and most of its second (issue #267, D-011, D-062): the sound system's
real-time side, its first spatial layer, footsteps and impacts by surface, and ambience by the
hour, the weather and the medium, all made in code, built and tested without a device, a window
or the GPU. The game starts and steers voices through an `AudioEngine`; the `Mixer` renders them,
wherever the output wants it: in a device's callback, on a mix thread of its own, or offline
into a buffer that the tests measure and the owner can listen to as WAV files. Nothing in the
library opens an audio device; the `audio-demo` example, which the owner starts, plays it live
(below).

```
cargo test --release -p forge-audio                                      # 66 tests on rendered samples
cargo test --release -p forge-audio --features steam-audio --test steam  # 5 more with the HRTF (the SDK, below)
tools/fetch-sounds.sh                                                    # the recordings, into assets/sounds/
cargo run --release -p forge-audio --example audio-render -- scenes      # WAVs in captures/audio/
cargo run --release -p forge-audio --example audio-render -- bench       # the mixer's cost
```

## How it works

| Part | How | Module |
|---|---|---|
| Commands | The game pushes commands into a wait-free single-producer, single-consumer ring (`rtrb`); each carries the frame it acts on. When the ring is full the engine keeps them, in order, and sends them at its next call. What the mixer is done with (a voice's sound, a stream's reader) comes back through a second ring with the voice's end and its reason, and is dropped on the game thread. The mixer never allocates, frees, locks or reads a file: every buffer is sized when it is made | `engine`, `mixer` |
| Clock | The mixer counts the frames it has rendered (`Clock`, an atomic). A play or a stop lands on its exact frame inside a block; a gain, a position or a bus level changes at the block's start and ramps across the block. A command for a later block waits in a list sorted by frame; one stamped with a frame already rendered acts at once and is counted late. A `TickClock` ties a simulation tick to a frame: 800 frames a tick at 48 kHz and 60 ticks a second, so an event stamped with a tick and a fraction of it plays on the same sample in every run | `clock` |
| Blocks | 256 frames (5.3 ms at 48 kHz), stereo, `f32`. The mixer renders a block whenever its output asks for more, so a device may ask for any size | `mixer` |
| Voices | A sound in memory (`SampleBuffer`, shared by `Arc`) or a stream, read at its own rate times the pitch and the Doppler factor through a four-point Hermite (Catmull–Rom) interpolator, exact on whole frames. An envelope gives fades in, stops on their frame (with or without a fade) and fades out | `voice` |
| The budget | At most `max_voices` (64 by default) are mixed. A new voice that finds the budget full takes the place of the weakest voice if it beats it: a voice already stopping first, then the lowest priority, then the quietest at the listener (its gain, distance curve and band gains). The weakest fades out over 5 ms beside the budget (16 such tails) instead of being cut. A voice marked `virtualize` (a sound in memory) goes virtual instead of ending: it is not mixed, but its time runs on, and when it is louder than the quietest real voice of its priority by 3 dB (or a place is free) it comes back where it would have been, fading in. Streams are never virtual: a stolen stream fades out and ends | `mixer` |
| Buses | A tree: every bus feeds its parent, and may send a share of its output (post-fader) to buses added before it, such as an aux bus with a reverb. They run from the last added to the master, so each has all its inputs first; a bus nothing played into is skipped. Faders and send levels ramp across a block | `bus` |
| The limiter | On the master, after its fader: the gain that brings each frame's peak to the ceiling (−1 dBFS), through a release follower (80 ms), its minimum held over a 64-frame look-ahead, then averaged over the same 64 frames, the audio delayed 63 frames. The gain applied to a frame is never above that frame's own target, so the ceiling holds by construction; a final clamp catches rounding and counts what it touches (0 in every test and scene). The mixer's latency is those 63 frames | `dsp::limiter` |
| Space | Positions in `f64` (D-004), taken relative to the listener. The direct path's direction, distance and gain go to a `Spatializer`: the default pans at equal power by the direction's lateral part (−3 dB a channel straight ahead, behind, above or below, like a centred mono sound), pulled to the centre inside the source's radius. The distance curve is OpenAL's clamped inverse model (−6 dB a doubling past the reference distance) or a line. Air absorption (Steam Audio's coefficients: −0.2, −1.5 and −15.8 dB at 100 m in the bands below 800 Hz, 800 Hz to 8 kHz and above) and occlusion (the share of the direct path a blocker covers, and the blocker's gain in each band) multiply into three band gains, applied as Steam Audio applies them: a low shelf, a peaking filter and a high shelf, the gains normalised, crossfaded when they change. Doppler from both velocities, eased over about 25 ms. A spatial voice's send falls with the square root of its distance curve, so a far voice is more of its room | `spatial`, `dsp::filter` |
| Reverb | On aux buses: a feedback delay network after Jot and Chaigne, eight lines of 21 to 60 ms (scaled by the room's size) through an orthogonal Hadamard matrix, each line's gain from the decay time and a one-pole damping filter that makes the highs die sooner, a pre-delay. Presets for a room, a hall, a cave and the open; parameters change while it runs | `dsp::reverb` |
| Materials | Steam Audio's eleven acoustic material presets from its `phonon.h` (`AcousticMaterial`: absorption, scattering and transmission in its three bands). `Occlusion::through(amount, layers)` multiplies the layers' transmissions, as Steam Audio does along a ray, and applies them to the occluded share of the direct sound. Through a wood door the bands fall by 23, 37 and 46 dB; through brick by 36 dB each; through concrete by 36, 54 and 60 dB. The world's material rows (D-007) are meant to carry these values; until they do, a game picks a preset per surface | `spatial` |
| Reverb zones | Boxes and spheres in the world, each with its reverb and a fade outside its shape. Where the listener stands, the zone with the highest weight wins (a later zone wins a tie, so a room inside a hall is added after the hall), and its parameters are blended with the outdoor ones by that weight, decay times in the log domain. `ReverbZones::follow` sends the result to a reverb bus from the game thread when it changes by more than 1 %. Sources in another room than the listener's (heard through a door) wait for the world model's rooms and portals | `zones` |
| Decoding | WAV in our own code (8, 16, 24 and 32-bit integers, 32 and 64-bit floats, plain or extensible, other chunks skipped); Ogg Vorbis through `lewton`, with the first page's padding trimmed by its granule position (FFmpeg's encoder leaves 704 frames that `lewton` keeps); Ogg Opus through `opus-rs`, a pure-Rust port of libopus, with the pre-skip and the end trimmed by the head and the last granule position. All pure Rust | `decode` |
| Streaming | `Streams::open` makes a ring of a quarter of a second between a decoder and the voice that plays it, fills it at once, and hands the decoder to a `StreamPump`: on a decode thread of its own, or called before each block offline (so an offline render is the same to the bit on any machine). A ring that runs dry plays silence for what is missing, keeps the voice's time and counts an underrun. A looping stream rewinds its decoder | `stream` |
| Output | `OfflineRenderer` renders seconds into a buffer as fast as it goes; `write_wav` writes 16-bit, 24-bit or float WAV. `MixThread` runs the mixer on a thread that fills an output ring ahead of the device, whose callback only copies out of it (`OutputRing::pull`, never waiting); the ring's length is the latency it adds | `render`, `thread` |
| Footsteps and impacts | `Foley::step` (a surface, a place, the walker's speed, weight and shoes) and `Foley::impact` (a surface, a place, a body's speed and mass) synthesise the sound on the game thread and play it as a spatial voice; `Foley::own_step` plays the listener's own feet without a place (straight below the head an HRTF has nothing measured), a little to the side of each foot. A contact is a `sin²` force pulse, short on hard ground (0.15 ms on metal, 0.35 ms on stone), long on soft (12 ms on gravel, 25 ms on grass); it drives the surface's modes (floorboards low and hollow, a steel plate for a second; stone's are damped within a millisecond and only colour the click), a click of noise, grains while the foot presses in (gravel's pebbles, grass's stalks), noise (grass brushing, water sloshing) and Minnaert bubbles (water). On hard ground the shoes count as much as the ground: the contact lasts as long as the softer of sole and ground lets it (a leather heel 0.3 ms, trainers 4 ms, bare feet 10 ms), a soft sole slaps in its own band, boots thud, sandals slap the heel as the foot lifts. A step is the heel then the softer ball, 85 ms apart walking, 22 ms running, and two steps in three a scuff as the toes push off. The amplitude is `√(weight × speed)` relative to 80 kg at 1.4 m/s; an impact's is `√mass × speed`, its contact widening with the mass and its click darkening. No two steps are alike, audibly: each draws its contact (±25 %), click (±3 dB), modes, balance, gap, scuff and level (±1.5 dB), and plays at its own pitch (±2.5 %). Each sound starts on 0, ends on 0 after a 5 ms fade, and keeps its top octave empty | `foley`, `dsp::synth` |
| Variation | For recorded sounds played again and again (a door, a pebble), `SoundSet` is the random container of Wwise and FMOD: takes in an order that never repeats the last few (half the takes, at most three), each at its own pitch (±3 %) and level (±1.5 dB). The recorded birds keep the same rule per species | `variation` |
| Ambience beds | Six textures made in code, each a decoder that never ends, streamed from the decode thread: wind (pink noise through a band that follows two wandering gusts, one per ear; leaves; a whistle past 6 m/s), rain (40 drops a second each mm/h: ticks on stone and bubbles in puddles; a wash), cicadas (up to 14 *Cicada orni*, each singing echemes of 80 ms every 230 ms near 4.8 kHz, in bouts), birds (a blackbird, a house sparrow, a serin, a collared dove), the sea against the quays (waves every 2 to 5.5 s, their gurgle, a splash above the biggest, a low wash), under water (a rumble, the diver's breath, far bubbles). Each reads one intensity the game writes through an atomic and eases to it over half a second; an eighth-order low-pass at 10.5 kHz keeps its top octave empty; asleep, it writes silence without working | `ambience::beds` |
| The place (#350) | `Habitat` in the state: `green`, 0 for streets and houses only to 1 for gardens and woods, and `indoors`, 0 to 1. The cicadas sing from trees: none below 0.1 of green, all from 0.5. The birds thin to a third among houses (sparrows and doves live there) and are all there from 0.4. Indoors, every bed of the outside is 16 dB down, the one under water untouched. The default is open country, the beds as they were. `BirdMix` (in `AmbienceAssets`) sets each species' share of the calls: blackbird 0.3, sparrow 0.3, serin 0.25, collared dove 0.05 by default. The dove used to take 0.15, and the owner heard its coo everywhere | `ambience` |
| The rules | Each tick `Ambience::update` takes the world's state where the listener is (the hour, sunrise and sunset, the temperature, the wind, the rain, how wet the place is, the nearest shore, the medium) and moves each bed's gain toward its target, rising and falling at their own pace (the wind 1 s both ways; rain 2 s; cicadas rise over 3 s and fall over 0.8 s, birds 3 s and 1.5 s; the sea 0.5 s; the medium 60 ms): cicadas from 2.5 to 4.5 hours after sunrise until sunset, above 22 °C; they stop within two seconds of rain and stay quiet while the place is wet (the ambience remembers the rain and dries over two minutes, or takes the simulation's `wet`), and give way to the wind from 5 m/s (silent by 10); the dawn chorus from an hour before sunrise to 2.5 hours after, 15 % of it through the day, 40 % at dusk; the sea −3 dB a doubling of the distance past 4 m (a quay is a line source), panned toward the water, its swell rising with the wind. A bed whose gain has rested at 0 for a second sleeps | `ambience` |
| Under water | The ambience's layout puts everything heard above the water under a `world` bus with a filter (`Control::BusFilter`): under water it low-passes at 600 Hz (fourth order), drops 29 dB (the surface's cost, research note §2.4) and narrows the image to a fifth. The cutoff glides a quarter of an octave a block (20 kHz to 600 Hz in 21 blocks, 112 ms), the gain (in decibels) and the width with a 30 ms time constant, through two state-variable sections that can be retuned while they run; clear, the filter is skipped. The underwater bed plays on a `water` bus that sends to its own reverb, sized for sound 4.3 times faster (`ReverbParams::UNDERWATER`: 1.4 s, dense, dark) | `dsp::medium`, `ambience` |
| The HRTF | With the `steam-audio` feature (Windows), `SteamSpatializer` replaces the panner: each voice slot has its own Steam Audio 4.8.1 binaural effect with the default HRTF and bilinear interpolation between its measured directions, through `audionimbus`. The voice's gain ramps across the block before the effect; inside the source's radius it blends toward the centre (`spatial_blend`); a voice that starts inside a block sits at its frame in a block-long buffer. A source behind sounds behind, one above above, which panning cannot do | `steam` |
| Recorded birds | When `tools/fetch-sounds.sh` has fetched the recordings, `BirdCalls::load` folds each to mono, resamples it to the mixer's rate, high-passes it above the species' song (1.2, 2 and 2.5 kHz) and cuts it where it stands 12 dB over its floor (gaps under 120 ms closed, calls under 60 ms dropped, over 3 s split at their quietest window, 15 ms edges, a peak of 0.5): 24 blackbird phrases, 10 sparrow calls, 22 serin passages. The birds bed sings them at their own place, distance (far ones dulled to 4 kHz) and pitch (±3 %), and plays the morning chorus far away while the dawn chorus is full, looped with a 2 s equal-power seam. The collared dove stays made in code (Commons has no free recording of it); without the files every bird is. The cicadas come from a public-domain recording of *Cicada orni* by the Hérault river (63 s), high-passed at 1.5 kHz: up to six singers each read it from their own place, at their own pitch (±4 %), place and distance (two in the middle distance first, the near one third, three far and dulled), in bouts of 15 to 60 s; the intensity sets how many sing, the loop's seam is a 2 s equal-power crossfade. Without it the cicadas are made in code | `ambience::recordings`, `ambience::beds` |

## Numbers (release build, Ryzen 7 9800X3D, one thread, 2026-10-16)

The mixer's work for one second of sound (`audio-render bench`): each voice a looping mono sound
at a pitch between 0.8 and 1.25 (so the interpolator runs), placed 2 to 40 m away and moving
(positions every 10 ms: distance, air, Doppler, the band filters, the pan), into a bus that, with
0.3 sent from every voice, feeds a hall reverb. The best of three runs of 10 s; three other
sessions shared the CPU.

| Voices | ms a second of sound | Share of a core | Mean block (5.33 ms) | Longest block |
|---|---|---|---|---|
| 0 | 0.27 | 0.03 % | 1.3 µs | 4 µs |
| 32 | 12.1 | 1.2 % | 64.5 µs | 137 µs |
| 64 | 24.6 | 2.5 % | 131 µs | 264 µs |
| 128 | 45.2 | 4.5 % | 241 µs | 418 µs |

About 0.35 ms a second a voice: 1.9 µs a voice a block, 7.4 ns a voice a frame. The row of 0
voices is the limiter and the buses alone (a reverb nothing feeds is skipped). 128 voices leave
95 % of a block's time. Runs while the other sessions build come out up to 25 % slower.

Decoding a second of stereo at 48 kHz on the decode thread: WAV (16-bit) 0.01 ms, Ogg Vorbis at
128 kbit/s 1.7 ms, Ogg Opus at 64 kbit/s 1.7 ms: a stream costs under 0.2 % of a core.

A walking step synthesised on the game thread (the mean of 200, leather shoes): stone 311 µs,
grass 421 µs, wood 495 µs, gravel 513 µs, water 611 µs, metal 853 µs. The beds on the decode thread, for a second of
sound: wind at 10 m/s 1.03 ms, rain at 20 mm/h 1.55 ms, the full cicada chorus 3.19 ms, the dawn
chorus 2.06 ms, the sea 1.24 ms, under water 0.85 ms: about 1 % of a core for all six at their
busiest (`docs/PROFILE.md`).

Steam Audio's HRTF, the same voices as above (the bench with `--features steam-audio`):

| Voices | ms a second of sound | Share of a core | Mean block | Longest block |
|---|---|---|---|---|
| 32 | 32.2 | 3.2 % | 171 µs | 308 µs |
| 64 | 79.7 | 8.0 % | 424 µs | 576 µs |
| 128 | 147.8 | 14.8 % | 788 µs | 1268 µs |

About 4.4 µs a voice a block more than the panner. Loading and cutting the five recordings takes
1.0 s on the game thread at start (decoding 5 min of Vorbis, the birds' chorus most of it).

## What the tests check (on rendered samples)

| Test | What must hold |
|---|---|
| A source on the left | 3 m left and 3 m ahead: the left channel is louder by the pan law's 12.6 dB to 0.1 dB; mirrored on the right |
| Two sources mix | 440 Hz at 0.3 and 1 kHz at 0.2: each channel's RMS is 0.707 × √((0.3² + 0.2²)/2) to 0.5 %; the same sound twice in phase is +6.02 dB to 0.01 dB |
| The limiter holds | A sine at 0.9 and noise at 0.8, +18 dB on the master: every sample ≤ −1 dBFS, the peak above 99 % of it, nothing clamped; quiet audio passes bit for bit, 63 frames late; a +12 dB square settles at the ceiling |
| Stealing by priority | Budget 4, priorities 5, 1, 7, 3, then a fifth at 6: the priority-1 tone is gone (below 10⁻⁴), the four others at their level to 0.002, one `Stolen` event |
| Stealing by distance | Budget 4 at equal priority, voices at 2, 16, 4, 8 m, then one at 3 m: the 16 m voice is stolen; a sixth at 50 m is refused |
| Scheduling | Clicks at frames 12 345, 12 800 and 30 001 (the last sent mid-render) come out on those frames plus 63, nothing else; a tick and a quarter through a `TickClock` lands on frame 4800 + 30 × 800 + 200; no command late |
| Stops | A stop at frame 10 001 without a fade: silent from that frame on; with a 480-frame fade, falling from that frame to silence 480 frames later |
| Virtual voices | Budget 1: a looping tone stolen by a short sound comes back after it, in phase with where it would have been to 10⁻³; stopped voices give way first and still report `Stopped` |
| Buses and sends | A bus at 0.5 sending 0.25 to an aux: 0.625 of the voice; the send and the aux at 0: 0.5 |
| Distance | 1, 2, 4, 8 m on the inverse law without air: −6.02 dB a doubling to 0.05 dB |
| Determinism | 40 voices of noise and tones at three rates, pitched, moving, stolen, virtual, through a reverb bus, the listener turning: two renders equal to the bit |
| The mix thread | 50 clicks sent from the game thread while the mix thread runs and a device thread pulls: all 50 on their frames, 50 `Finished` events back, none late |
| A full command ring | A ring of 4 and 20 plays: 16 wait, all start in order, their events in order |
| WAV | 16-bit, 24-bit and float round trips within half a step; 8-bit and 32-bit files with an odd `LIST` chunk before the data |
| Streams | A stereo 44.1 kHz WAV streamed and pitched plays the same samples to the bit as from memory; a looping stream matches a looping sound to 10⁻⁴; a starved ring plays its 0.1 s then silence and counts underruns |
| Ogg Vorbis and Opus | The fixtures (a tone made here, encoded by Blender's FFmpeg with libvorbis at 128 kbit/s and libopus at 64 kbit/s): 48 000 frames exactly, each side's level to 2 %, crosstalk below 0.002, the waveform within 30 dB (Vorbis) and 20 dB (Opus) of the source; the same after a rewind; Vorbis streamed equals Vorbis in memory to the bit; Opus loops as a stream |
| The DSP | The reverb's decay measured by Schroeder's backward integral within 15 % of 0.5, 1.2 and 2.5 s; the band filter keeps 150 Hz within 1 dB and cuts 12 kHz and 16 kHz by more than 12 and 20 dB for a high band of 0.05 |
| A wall | Tones of 150 Hz, 2.5 kHz and 16 kHz 5 m ahead, behind a wood door: they fall by 23.6, 37.5 and 46.7 dB against the wood's 23.1, 37.1 and 46.0, each within 1 dB; half hidden behind brick, −5.9 dB in every band to 0.2 dB |
| Air | The same tones at 100 m against 1 m, no distance curve: −0.21, −1.63 and −15.4 dB against Steam Audio's −0.17, −1.46 and −15.7 (within 0.1, 0.3 and 0.5 dB) |
| Doppler | A 1 kHz tone 200 m ahead: 1111.11 Hz when it comes at 34.3 m/s, 909.09 Hz when it goes, 1100.00 Hz when the listener comes, each within 10⁻⁴ |
| Reverb zones | A hall (a box 20 m across, 5 m of fade) in the open: the reverb's low-band decay measured on the rendered tail is 2.37 s inside (the hall's 2.4), 0.93 s half-way through the fade (0.92, the log-domain blend), 0.36 s outside (the open's 0.35), each within 5 % |
| Surfaces | A walking step on each of the six surfaces in leather shoes, six seeds each: two steps on one surface at most 3.9 dB apart (RMS over nine octaves), every pair of surfaces further apart than 5 dB and than 1.5 times that (the closest, gravel and grass, 7.8 dB); wood's centroid under 400 Hz, water loudest in the 500 Hz to 1 kHz octave, stone's 4 and 8 kHz octaves within 4 dB of its loudest (a click, not a tone), gravel and metal above 1.5 kHz, grass in 4 to 8 kHz; metal 40 dB down after more than twice any other surface's time |
| Shoes | On stone: leather's centroid more than twice trainers' and three times bare feet's (1064, 383, 249 Hz); boots more than 6 dB above leather under 250 Hz (14 dB measured); bare feet the quietest; a sandal's slap 120 to 200 ms after the ball, 6 dB over a trainer there. On grass the shoes move the centroid by under 15 % (the ground decides) |
| Steps that differ | Twelve steps in a row on stone: no two alike, their centroids spread more than 20 %, their levels over 2 dB |
| Weight and speed | 40, 80 and 160 kg: +3.01 dB a doubling to 0.01 dB; 4.2 m/s against 1.4: +4.77 dB; the heel and the ball 68 to 102 ms apart walking (where the level jumps most), 15 to 30 ms running; impacts +6.02 dB for four times the mass or twice the speed, a 20 kg body on stone, wood or metal darker than a 1 kg one (its centroid under 0.8 times) |
| No clicks (footsteps) | Each sound starts and ends on exactly 0; under −38 dB of its energy above 16 kHz and no millisecond above −25 dB there, walking and running; the same sound cut at its peak without a fade scores at least 10 dB worse, so the measure sees a click. Twelve steps through the mixer, each on its frame (the first contact within 1.5 ms), a step 3 m to the right louder on the right by more than 6 dB, the same samples twice |
| Cicadas in rain and wind | Two seconds into a rain of 6 mm/h the cicadas' gain is under 0.1; a minute after it stops the place is still wet and they are silent; five minutes later they sing again; the rules give them nothing at 10 m/s of wind and under 15 % at 9 |
| Cicadas by the hour | At 13:00 the 3 to 8 kHz band at 30 °C (−23.2 dBFS) is 31 dB above the same hour at 18 °C (birds alone); at 3:00 and 21:30 nothing plays; a lone cicada's envelope repeats every 275 ms (its species: 130 to 450 ms), its loudest octave 4 kHz |
| The dawn chorus | The 1 to 8 kHz band at 6:30 (−34.2 dBFS) above 20:18 (−39.1) above 13:00 (−43.9), each by more than 2 dB; the rules give the birds nothing in 5 mm/h of rain, in 12 m/s of wind, at 2:00 |
| Wind and rain | Wind at 3, 8 and 15 m/s: −30.9, −18.2 and −10.3 dBFS, its centroid 297, 551 and 892 Hz, its level in 100 ms windows wandering by 5 dB, the ears correlated under 0.01. Rain at 20 mm/h against 2 mm/h: +10.03 dB |
| The sea | The shore at 4, 16 and 64 m: −6.02 dB every four times the distance to 0.05 dB (−27.6 dBFS in the near ear at 4 m), the water's side 10.3 dB louder, nothing past 400 m |
| Under water | A 400 Hz tone in the world's bus falls 33.6 dB (29 for the surface, 4.4 as the width narrows, 0.17 for the low-pass) to 0.3 dB; a 3 kHz tone falls 81.5 dB, and by more than 40 dB within 150 ms of the dive; a tone hard left reaches the right ear at 0.674 of the left (0.667 for a fifth of the width); the rumble under 150 Hz rises 19.8 dB; the water bus's reverb decays in 1.4 s within 15 % |
| A day in time-lapse | 30 s: the hour from 4:00 to 14:00, a wind to 12 m/s, a shower, the walk down to the quay, a dive from 20 to 25 s: every bed rises above half its level, no millisecond above −25 dB over 16 kHz (the worst −35.5; −16.8 before the filter became a state-variable one), the peak under the limiter's ceiling, the same samples twice |
| Sleeping beds | The rain stops: its bed sleeps once its gain has rested at 0 for a second and writes silence; the rain comes back: it wakes and fades in, no click |
| Recorded calls | With the recordings: at least 15 blackbird, 5 sparrow and 10 serin calls (24, 10, 22), each 0.06 to 3.05 s, starting and ending on 0, a peak of 0.5, centroids in their species' band (blackbird 1.5 to 4.5 kHz, sparrow 3 to 4.5, serin 3.5 to 8); the morning chorus over 170 s; an empty folder gives no recordings and no error. Skipped, with a note, without the files (CI) |
| The recorded cicadas | With the recording: over 60 s of song centred at 5.2 kHz (the species' peak 4.82 ± 0.49 kHz); one singer's envelope repeating every 140 ms (the species: 130 to 450 ms); the full chorus 5.8 dB above one singer (−24.7 against −30.5 dBFS) and less alike between the ears; no millisecond over −25 dB above 16 kHz; the same samples twice. Skipped without the file |
| The recorded dawn | The birds bed with the recordings: the 1 to 8 kHz band at the dawn chorus (−23.2 dBFS) 17.7 dB above the day (−40.9), no millisecond over −25 dB above 16 kHz, the same samples twice |
| The HRTF (feature `steam-audio`) | A source 3 m to the left: the near ear 16.9 dB louder between 2 and 8 kHz, the far ear 0.73 ms later (the lows' cross-correlation); ahead and behind 3.2 dB apart over the third octaves of 1 to 20 kHz (7.8 dB at most), ahead and above 3.8 dB, where the panner gives the same samples for ahead and behind; −6.02 dB a doubling of the distance to 0.01 dB; straight ahead 1.4 dB over the panner; a source circling at 4 m no worse above 16 kHz than one held still (−21.5 against −21.1 dB: the HRTF itself lifts the top octave in front and behind), the same samples twice; a click on its frame, then the HRTF's own 65 samples |

## The scenes (`audio-render scenes`)

Rendered offline into `captures/audio/*.wav` (16-bit, 48 kHz, ignored by git) for the owner to
listen to; each line prints the voices, steals and the limiter's work.

| Scene | What to listen for |
|---|---|
| `orbit` | Rain on a tin roof (noise in bursts) circling at 4 m every 4 s, a tone straight ahead. Panning cannot tell front from back: the rain behind sounds as it does in front |
| `steal` | 16 tones into a budget of 6, one every quarter second, then one stopping every half second from 4.5 s: the low priorities give way, then come back where they would have been |
| `doppler` | A buzz passing 4 m ahead at 30 m/s, left to right: up 10 % on the way in (343 / 313), down 8 % on the way out (343 / 373) |
| `limiter` | Clicks and bursts 18 dB over full scale, held under −1 dBFS (15 dB of reduction at the deepest, nothing clamped) |
| `stream` | A 2 s stereo WAV made in memory at 44.1 kHz, streamed and looped while a tone plays from memory |
| `wall` | Rain and a buzz 6 m ahead: a wood door closes over a second at 1.5 s and opens at 4.5 s (the sound dulls and drops, the low end stays longest), then a concrete wall slides across at 7 s (almost nothing gets through) |
| `zones` | The listener walks at 5 m/s from the open, through a stone hall, out again and into a cave, clapping every 0.6 s: the tail grows through the hall's 6 m of fade, falls back outside, and grows longer and brighter in the cave |
| `footsteps` | The walker's own feet, played without a place: 3.3 s walking in leather shoes on each of stone, gravel, grass, wood, metal and water, then running back over them; a 20 kg crate onto boards, a stone onto a steel plate and a sack into water, 3 m ahead; then stone again in trainers, boots, bare feet and sandals |
| `village` | A day in a minute, 3 s an hour from 3:00: night, the dawn chorus, cicadas from mid-morning, wind and a shower in the afternoon, a walk down to the quay from 30 s (the sea grows on the right), a dive from 45 to 50 s (the world goes low, quiet and narrow, the rumble and bubbles come up), and the evening |
| `orbit-hrtf` | With `--features steam-audio`: the orbit through the HRTF, for headphones; the rain circling behind now sounds behind |

## Sound in the app (#318)

forge-app owns the output device (`forge_audio::device`, the `device` feature: `cpal`, WASAPI,
Windows only for now). A demo that asks for sound (`AppConfig::sound`; the galleries do, the city
plan and the village among them) opens the default device at its own rate when the run is
interactive; **scripted runs never do**: a frame limit (`--frames`), a capture, `--no-sound` or
`FORGE_SOUND=0` keep it silent. The console's `snd.volume` (0 to 1) is the master's level, and
`snd.volume 0` in `forge.cfg` silences every run.

**Recording instead of playing** (`--sound-record FILE.wav`, `AppConfig::sound_record`): the same
buses and mixer at 48 kHz, rendered a frame's time at the end of each frame into a float WAV
written at exit: the demo's step with `--fixed-step` (`AppConfig::sound_record_step`), so the sound
keeps the scene's time; the wall clock's otherwise. Scripted runs may record; the device is never opened. It is how a session
checks what a scene sounds like without playing it on the owner's machine.

- **The village at its house view** (`--view=76.6,1.7,-184.3,146.8,0`, 15:00, green 1.0):
  −25 dBFS RMS, the cicadas at full (2026-10-23). The mix plays there; if a run is silent, look
  for the log's `sound` line (the device and its rate) or `no sound: …` at the start.

- **What the game gets** (`Context::sound`): the engine to play voices on, the buses (the
  ambience's, `effects` and `foley` under the world's bus and sending to its reverb, `ui` under
  the master), the stream opener for beds and music (decoded on the shell's thread), the voices
  that ended each frame, and `listen` for the camera.
- **The galleries** (`city_blocks::world_sound`): the ambience's beds at the sun's hour (`--day`,
  `--time-of-day`; mid-afternoon otherwise), the player's footsteps every 0.75 m walked (1.1 m
  running) on what the scene says is underfoot (`Running::footing`; the island's walker on its
  ground's row since 2026-10-24: sand and grass as grass, rock stone, gravel and scree gravel),
  and the scene's own sounds (`Running::sounds`).
  - **The place** (#350, `Running::habitat`): how green the ground within earshot is and how far
    indoors the camera stands. The beds follow it over 1.5 s (open country when a scene gives
    none).
  - **The birds** (#350, `Running::bird_mix`): the scene's species and their shares, read once
    when the beds start (the Mediterranean village's when a scene gives none).
  - **The recordings** are read from the engine's `assets/sounds/` whatever the working
    directory. A game run from its own folder used to get the beds made in code.
- **The village's place** (city-plan, #350): the share of the ground within 32 m under the crowns
  of its trees and its forest's plants, 30 % counting as a wood; indoors inside a room's volume.
  - **In a street:** the cicadas fall silent and a third of the birds call.
  - **In the forest:** both sing as before.
  - **The dove** calls only near woods (none below 0.15 of green, all its share from 0.5), about
    5 dB under the other birds: the owner heard it too often and too near (2026-10-23).
  - **In a house:** the outside's beds are 16 dB down.
- **The village** (city-plan): boards indoors (inside a room's region); outdoors the ground's
  section under the foot, as its row's sound layer says (D-007, 2026-10-24): the open land, the
  gardens and the parks grass, the paving and the quays stone, the water mirrors water. The
  sections come from a map at a metre painted from the tiles' ground at load (4 ms for the
  600 m village; none for the city, whose streets stay stone). With `--pursuers`, their boots, each step muffled by the walls and floors between it and the camera
  (`Pieces::between`: brick's transmission per layer, at most three) and at most six mixed at once.
- **The physics lab's impacts** (Phase 7, 2026-10-23): each contact a tick begins sounds as its two
  bodies' material rows (`forge_core::material::SoundLayer`, D-007; `docs/demos/physics-lab.md`,
  "Impacts by material").
  - **Two bodies meeting** (`Impact::between`): each with a surface rings, both driven by the
    pair's reduced mass, `m₁m₂ / (m₁ + m₂)` (a body on fixed ground: its own).
  - **The softer sets the contact** (`Impact::against`), as a soft sole does on hard ground: it
    lasts as long as the softer lets it, and the click is the softer's own, none for grass or
    gravel. A steel plate's modes from 3 to 9 kHz, struck at 2 m/s by 2 kg: −29.7 dBFS by steel,
    −53.3 by a wooden crate, nothing (−126) by a sack (the test
    `two_bodies_meeting_ring_each_other`).
  - **Rubber, cloth and flesh** have no surface of their own (`SoundLayer::surface` is `None`):
    only what they strike rings.
- **A cap per family of voices** (`PlayParams::family`, `AudioEngine::set_family_cap`): past it
  a new voice takes the place of its family's weakest when it beats it (priority, then loudness at
  the listener), and is refused or kept virtual otherwise; the other families keep their places.

To hear it: `cargo run --release -p city-plan -- --village 600 --pursuers --view=77.8,1.7,-184.4,146.8,-5`,
Enter to walk (the owner's check
by ear, 2026-10-21).

## Hearing it live: `audio-demo` (the owner starts it)

The audio crate's own program, the whole village of sounds driven from the terminal. It is an
example behind the `device` feature, which brings `cpal` (WASAPI) on Windows only, and opens the
device the same way the app does; no test, script or session runs it.

```
cargo run --release -p forge-audio --features device --example audio-demo -- --tour
cargo run --release -p forge-audio --features device --example audio-demo             # commands only
cargo run --release -p forge-audio --features device --example audio-demo -- --list  # the devices
cargo run --release -p forge-audio --features device,steam-audio --example audio-demo -- --tour --hrtf
```

It plays on the default output device at the device's own rate and sample format (f32, i16 or
i32), stereo into the first two channels. The mixer runs on its mix thread ahead of a ring of
1024 frames (21 ms at 48 kHz, `--ring` to change it), the beds on the decode thread, and the
device's callback only copies out of the ring. The terminal drives the village of #266:

| Command | What it does |
|---|---|
| `time <h>`, `lapse <h>` | the hour; hours that pass a minute |
| `temp <C>`, `wind <m/s>`, `rain <mm/h>` | the weather |
| `sea <m>`, `sea off` | the distance to the quay, on the right |
| `walk <surface>`, `run <surface>`, `shoes <kind>`, `stop` | the walker on stone, gravel, grass, wood, metal or water, barefoot or in sneakers, leather, boots or sandals |
| `drop <surface> <kg> <m/s>` | something falls 3 m ahead |
| `dive`, `up` | the head under water, or out |
| `volume <dB>`, `status`, `tour`, `help`, `quit` | the master's level, the state and the mixer's counters, the tour |

A line `now: <bed> starts` or `stops` names each sound as it comes and goes. `--tour` (or `tour`)
plays 160 s: night at 3:00, the dawn chorus, a walk over each surface, stone in every pair of
shoes, a run, noon at 31 °C with the cicadas, wind, a shower, the walk to the quay, wading, a dive, three
falls, dusk and night. `--seconds <s>` stops it after that long. On quitting it prints the
mixer's counters and how often the device found the ring short.

### Setting up the HRTF and the recordings

- **Steam Audio's SDK** (for `steam-audio`): `steamaudio_4.8.1.zip` from Valve's GitHub releases
  (181 171 027 bytes, SHA-256 `4a0aa5ec1176f38f0b0993a37c2259d9e86f27e22d5e24f83ec4c3cb9a1d5449`),
  its `steamaudio/include/`, `steamaudio/lib/windows-x64/` and `THIRDPARTY.md` unpacked into
  `steam-audio-sdk/` at the repository's root, which git ignores. `.cargo/config.toml` points
  `STEAMAUDIO_LIB_DIR` there; forge-audio's build script copies `phonon.dll` next to what cargo
  runs.
- **LLVM** (for `steam-audio`): `audionimbus-sys` generates its bindings with `bindgen`, which
  needs `libclang`; LLVM's Windows installer (`winget install LLVM.LLVM`) puts it where `bindgen`
  looks. CI's Windows runner has it.
- **The recordings**: `tools/fetch-sounds.sh` fetches the four birds' into `assets/sounds/birds/`
  and the cicadas' into `assets/sounds/cicadas/`, and checks their sizes and SHA-1
  (`assets/sounds.tsv`).

## Not yet

- **The device, further.** The mix thread's priority (MMCSS "Pro Audio" on Windows needs a call
  the crate's `forbid(unsafe_code)` keeps out); the device's wake-ups measured; ALSA and CoreAudio
  (cpal has them; the Linux CI lacks ALSA's headers).
- **The data layer** D-011 names: events, RTPCs, states and switches, HDR loudness on the buses,
  banks, and the profiler's overlay (the counters exist: `MixerStats`).
- **Steam Audio, further:** its occlusion and reflections; the ambisonic bus; SADIE II's HRTF
  (D-011's pick, a download of its own); the HRTF only for the nearest voices, the far ones panned,
  if 8 % of a core for 64 voices matters.
- **The rest of the second part** of #267: the acoustic values in the world's material rows
  (D-007; the footstep and impact surface is there since 2026-10-23) and occlusion found by rays
  against the world, sources heard from another room, rolling and sliding, the wind sampled from
  the vegetation's wind field, recordings to fit the modal banks (Ren et al.; a download).
- **Surfaces of their own for sand, mud, snow and ice:** their rows borrow grass's dull press,
  gravel's grains and stone's ring for now.
- **Sounds made off the game thread:** an impact or a step is synthesised where it is asked for,
  0.1 to 1 ms each (10 to 40 ns a sample, a heavy body's ring lasting up to a second). The lab
  makes one meeting a frame for that reason (0.37 ms a frame on average while its pile comes
  down); a worker making them a frame ahead would lift the limit.
- **The ambience's sources.** The beds are textures around the listener; birds and cicadas
  placed in the map's trees, and the sea along its actual quays, wait for the map's data.
- **Better resampling** for sounds far from the mixer's rate: the Hermite interpolator aliases
  when it reads faster than one frame a frame; `rubato` at load time is the research note's pick.
