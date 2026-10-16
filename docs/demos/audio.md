# Audio — the mixer, rendered headless (`forge-audio`)

Phase 6's first step and the start of its second (issue #267, D-011, D-062): the sound system's
real-time side and its first spatial layer, built and tested without a device, a window or the
GPU. The game starts and steers voices through an `AudioEngine`; the `Mixer` renders them,
wherever the output wants it: in a device's callback, on a mix thread of its own, or offline
into a buffer that the tests measure and the owner can listen to as WAV files. Nothing in the
crate opens an audio device.

```
cargo test --release -p forge-audio                                      # 40 tests on rendered samples
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

## Not yet

- **A device.** `cpal` behind a cargo feature, in a binary the owner starts; the mix thread's
  priority (MMCSS "Pro Audio" on Windows needs a call the crate's `forbid(unsafe_code)` keeps out).
- **The data layer** D-011 names: events, RTPCs, states and switches, HDR loudness on the buses,
  banks, and the profiler's overlay (the counters exist: `MixerStats`).
- **Steam Audio** as a `Spatializer` (HRTF), its occlusion and reflections; the ambisonic bus.
- **The rest of the second part** of #267: the acoustic values in the world's material rows
  (D-007) and occlusion found by rays against the world, footsteps and impacts by material,
  weather and medium ambience, underwater, sources heard from another room.
- **Better resampling** for sounds far from the mixer's rate: the Hermite interpolator aliases
  when it reads faster than one frame a frame; `rubato` at load time is the research note's pick.
