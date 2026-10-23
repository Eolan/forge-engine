# Net — the transport, its protocol, the link conditioner and replication (`forge-net`)

Phase 5's first two steps (#268, D-010, D-062's parallel sessions): the networking, built and
tested headless on loopback, never touching the GPU or a window. A server and its clients talk
QUIC through a link conditioner that loses, delays, duplicates and reorders their packets from a
seed; the server replicates a crowd to them in snapshots under a byte budget ("Replication"
below). Everything binds 127.0.0.1.

```
cargo test --release -p forge-net
cargo run --release -p forge-net --bin net-loopback -- --clients 8 --seconds 10
cargo run --release -p forge-net --bin net-replicate -- --entities 3000 --clients 100
cargo run --release -p forge-net --bin net-loopback -- --clients 100 --replicate 3000
```

## How it works

| Part | How | Module |
|---|---|---|
| Transport | QUIC (RFC 9000) through `quinn` 0.11 on tokio, TLS 1.3 (RFC 9001) from `rustls` 0.23 with `ring` 0.17's cryptography. `ring` rather than rustls's default `aws-lc-rs`: quinn's own default, Ed25519 for the tokens from the same crate, and it builds with no CMake or NASM (its Windows assembly comes as objects; `cc` finds MSVC through the registry, with no `cl.exe` on PATH). The dependencies build cold in 18 s | `endpoint` |
| Certificates | a server shows a certificate; the client trusts exactly the one it was given, as SSH pins a host key (D-010's trust on first use). Tests and the headless binary make a self-signed one at run time (`rcgen`); none is stored | `endpoint::ServerIdentity` |
| Protocol | ALPN `forge-net` names it in TLS; the client's hello on the first stream carries the version (1), its channels and its token, checked in that order; the server answers with the client's id or closes the connection with a refusal code and reason (the version: both versions; the token: why) | `protocol`, `wire` |
| Tokens | netcode.io's connect token (client id, issue and expiry times, the servers it is good for, user data) signed with Ed25519 by the login service; a server holds only the public key, checks it offline, and cannot mint one. It travels inside TLS, so nobody on the path reads or replays it | `token` |
| Channels | numbered, declared alike by both ends: reliable ordered (a stream per channel, each message after its length as a QUIC varint), reliable unordered (a stream per message), unreliable (QUIC datagrams, RFC 9221). `Session::send` never waits: an ordered message joins its channel's queue, which a task writes, coalescing what is queued | `channel`, `session` |
| Fragmentation | an unreliable message longer than a datagram (1200 bytes at most, what every path takes) is cut into up to 255 fragments with a 5-byte header (channel, message number, index, count); the receiver joins them, and drops a message whose fragments have not all come within a second | `fragment` |
| Liveness | QUIC's pings after `keep_alive` without sending (2 s by default) and its idle timeout (10 s): a dead peer or a cut link ends the session with `TimedOut` on both sides | `endpoint::TransportSettings` |
| Link conditioner | per packet, in order: loss; a rate with a queue that drops what would wait too long; latency plus a uniform share of jitter (jitter alone keeps the order, as one path does); a share held back so later packets overtake it; a share duplicated. Five draws a packet from a seeded SplitMix64 whatever its fate, so the same seed and packets give the same fates. It sits under quinn's socket (`AsyncUdpSocket`), so the tests run the real protocol; each end conditions what it sends | `conditioner`, `socket` |
| Sockets | send and receive buffers of 4 MiB: Windows' default 64 KB overflowed under a burst of datagrams before the endpoint read them (the fragment test lost 15 messages in 50 on a perfect link before) | `endpoint` |

## What the tests show

`cargo test --release -p forge-net`: 23 unit tests (the conditioner's rates, order and
queue; fragments in any order; tokens; the handshake's bytes; bits, quantisation, deltas, the
interest grid), 11 on loopback (the last one replication's) and 5 of replication on a virtual
clock, 15 s in all. The transport's:

| Test | Link | What holds |
|---|---|---|
| each channel both ways | perfect | the client's id comes from its token; every channel carries messages both ways |
| the ordered channel's order | 20 ms, 10 ms jitter, 5 % lost, 2 % duplicated, 10 % held back 15 ms, both ways | 2000 messages of 64–463 bytes arrive complete and in order |
| the unordered channel | the same | 300 messages arrive, each once |
| unreliable losses | 10 % lost one way | 1000 datagrams of 1000 bytes: 89.4 % delivered, the link lost 10.5 % of 1016 packets (tolerance ±4 %) |
| fragments | perfect, then 10 % lost | 50 messages of 5000 bytes arrive whole; over the lossy link 55 % of 300 arrive (five fragments: 0.9⁵ = 59 %), none damaged, the unfinished ones dropped and counted |
| a dead peer | cut both ways after connecting | both sides close with `TimedOut`, after the 1 s idle timeout |
| keep-alives | perfect, silent for 3.6 s | a session with a 1.2 s idle timeout and 250 ms pings is still open |
| refusals | perfect | another version (told both versions), a forged token, an expired one, one for another server, garbage, other channels: each refused with its reason, on both sides; a good client is let in after them |
| an untrusted certificate | perfect | TLS fails; the server never sees a hello |

## Numbers

`net-loopback`, release, one thread for the server and every client, loopback through the
conditioner both ways. The server sends each client a message on the unreliable channel at the
snapshot rate (a snapshot's stand-in); each client sends 64 bytes at 60 Hz (its inputs). 10 s,
counted after every client is in. "UDP payload" is what quinn sends, its headers, acks and
encryption included; IPv4 and UDP add 28 bytes a datagram.

| Clients | Down | Link | Down per client | Up per client | Delivered | Round trip |
|---|---|---|---|---|---|---|
| 8 | 800 B at 30 Hz | 50 ms, 2 % | 25.3 kB/s UDP payload, 26.8 with IPv4, in 53 datagrams/s, for 23.5 kB/s of messages (+8.0 %) | 6.0 kB/s UDP payload, 7.7 with IPv4, in 62 datagrams/s, for 3.8 kB/s (+56 %) | 98.1 % | 124.8 ms |
| 100 | 800 B at 30 Hz | 50 ms, 2 % | 25.3 kB/s (+7.9 %) | 6.0 kB/s (+56 %) | 98.1 % | 123.7–125.7 ms |
| 8 | 800 B at 30 Hz | perfect | 25.9 kB/s (+8.0 %) | 6.0 kB/s (+57 %) | 100 % | 1.1 ms |
| 8 | 800 B at 20 Hz | 50 ms, 2 % | 17.2 kB/s (+10.5 %) | 5.9 kB/s | 98.0 % | 125.4 ms |
| 8 | 800 B at 60 Hz | 50 ms, 2 % | 49.0 kB/s (+5.0 %) | 6.3 kB/s | 97.9 % | 124.8 ms |
| 8 | 3000 B at 30 Hz (3 fragments) | 50 ms, no loss | 93.8 kB/s (+4.5 %) | 6.7 kB/s | 100 % | 123.3 ms |
| 8 | 3000 B at 30 Hz | 50 ms, 2 % | 81.5 kB/s (+8.4 %) | 6.3 kB/s | 85.6 % | 123.4 ms |

What the numbers say:
- **QUIC's cost on a snapshot of 800 bytes is about 8 %**, a little more at 20 Hz: of the 53
  datagrams a second the client receives at 30 Hz, 23 carry only acks for its 60 Hz inputs. A
  64-byte input costs 36 bytes more (+56 %): a short header, the connection id, the packet
  number, the AEAD tag, the datagram frame and the channel byte.
- **D-010's 24 kB/s a client** (the 100 bots' target) holds 800-byte snapshots at 30 Hz as
  messages, but 26.8 kB/s on the wire with IPv4: snapshots of about 720 bytes fit it whole.
- **Congestion control caps datagrams on a lossy, long link.** At 2 % loss and a 124 ms round
  trip, the window allows about 80 kB/s a connection; 3000-byte messages at 30 Hz (90 kB/s)
  wait in the datagram queue and the oldest are dropped, so 85.6 % arrive where the link's
  losses alone would leave 94 % (0.98³). A snapshot budget stays well under that.
- **The round trip reads 124 ms for a link of 2 × 50 ms plus up to 5 ms of jitter each way.**
  The conditioner's delays end on the runtime's timer, which on Windows wakes on the system
  timer's 15.6 ms tick; the fates do not depend on it.

## Replication

Step 2: the server sends each client a snapshot, one unreliable datagram, at the snapshot rate;
the client acknowledges what it took in each of its inputs.

| Part | How | Module |
|---|---|---|
| Bits | a hand-written bit writer and reader (D-010): values of any width least significant first, zigzag for signs, groups with a continuation bit for counts and gaps; reading past the end fails rather than panics | `bits` |
| Quantisation | positions in 1 cm steps of the f64 world position (D-004), sent whole as a 64 m cell (from the client's) and a 13-bit offset; orientations as smallest three, 2 + 3 × 9 = 29 bits, under 0.5° of error; velocities in steps of 1/32 m/s; two 16-bit words (what the entity is, what it does). Both ends hold the same integers, and only IEEE's correctly rounded operations make them (D-016) | `quantize` |
| Deltas | against the last state of the entity the client acknowledged, up to 31 packets back (a 5-bit field; older means a full state): five bits for what changed, then a position's three differences in 2-bit size classes (0, 6, 12, 24 bits) and a velocity's likewise. A walker going straight costs about 4.6 bytes, its full state about 21 | `snapshot` |
| Slots | a block names its entity by a slot the server gives it for this client (the lowest free one), so the blocks, sorted by slot, cost a few bits each for their gap where a city's ids would cost 20 or more; a full state carries the entity's id. Removals name ids, so one sent again after the slot went to another entity removes the right one | `snapshot`, `replicate` |
| Interest | cells of 32 m on the ground, rebuilt each tick; a client hears of the entities within 150 m of its focus, and lets go of one 10 m further out. The cells are the server's: no entity carries a query (Improbable's patents, research note §9) | `interest` |
| Priority | each entity in a client's interest adds its importance over `1 + (d / 20 m)²` every snapshot; the highest go first under the budget (720 bytes: D-010's 24 kB/s at 30 Hz once QUIC's and IPv4's headers are paid); one sent drops to zero, one the client already holds costs nothing | `replicate` |
| Initial states first | while an entity in a client's interest has never had its full state sent, or its last one went unacknowledged for 8 packets, a packet carries full states only. So no update to one entity goes out before another's first state: the order Verizon's US 11,810,245 claims never occurs (research note §9; the owner to confirm) | `replicate` |
| The client | decodes all or nothing, drops a packet older than the newest it took (so a removal and a return cannot swap), keeps its last 32 packets for baselines, and acknowledges the newest with a 32-bit field of the ones before it (6 bytes) | `snapshot::SnapshotReceiver` |
| A crowd | the stand-in the tests and the measures replicate: people in a square, half walking at 1.2–1.6 m/s, a quarter strolling, a quarter standing, turning every 8 s or so, ids spread out (97 apart on average) | `crowd` |

### What the tests show

`tests/replication.rs` runs the replicator and a receiver over two seeded conditioners on a
virtual clock (50 ms, 10 ms of jitter, 5 % lost, 2 % duplicated, 5 % held back, both ways);
`tests/loopback.rs` adds one over QUIC.

| Test | What holds |
|---|---|
| a client converges over a bad link | 3000 people, 20 s moving then 4 s still: the client holds exactly the server's integers for everyone within 150 m and nobody beyond 160 m; no snapshot over 720 bytes; the join is full states only, and no packet carries a delta while a first state waits |
| the same bytes | two runs of the same world and link send the same 150 snapshots, byte for byte |
| near before far | under 300 bytes a snapshot, people within 30 m are updated 8.8 times a second, those at 100–135 m 1.0 |
| leaving and returning | 100 of 200 people go 1 km away and come back: the client forgets them and holds them again, their slots reused meanwhile |
| a lost removal | a person leaves as another takes its slot, that packet is lost, and the first comes back under a new slot: the client still holds all three with their states (a receiver that trusted the old slot forgot the first) |
| over QUIC | 400 people over the bad link both ways for 7 s, the acks riding the client's datagrams: every snapshot decodes and the client ends with the server's state |

### Numbers

`net-replicate`, release, one thread, 3000 people in a 600 m square, the clients' focuses spread
over it, 50 ms and 2 % loss each way, 20 s (the first 2 s, the join, counted apart). About 490
people are in a client's interest.

| Snapshots | Budget | Bytes a snapshot (mean / p99) | Per client, payload / with QUIC and IPv4 | Updates a second within 30 m / 30–75 m / 75–150 m |
|---|---|---|---|---|
| 20 Hz | 720 B | 706 / 713 | 14.1 / 15.3 kB/s | 15.3 / 9.9 / 2.8 |
| 30 Hz | 720 B | 706 / 713 | 21.2 / 23.0 kB/s | 22.8 / 15.1 / 4.3 |
| 60 Hz | 720 B | 706 / 713 | 42.4 / 46.0 kB/s | 45.3 / 30.4 / 8.6 |
| 20 Hz | 1140 B | 1126 / 1133 | 22.5 / 23.7 kB/s | 15.3 / 15.4 / 5.6 |
| 60 Hz | 340 B | 327 / 333 | 19.6 / 23.2 kB/s | 34.3 / 12.4 / 3.2 |

- **A snapshot carries about 153 deltas** of 4.6 bytes; about 210 people who moved wait their
  turn each time, which is the accumulator doing its work: near people every packet or two,
  far ones a few times a second.
- **At the same 23–24 kB/s on the wire,** 30 Hz serves the near and middle bands best; 60 Hz
  pays a third more in headers for the nearest and starves the rest, 20 Hz the reverse.
- **The join** takes 13–18 snapshots of full states (0.5 s at 30 Hz) before the first delta.
- **Everyone in every snapshot** (`--everyone`, no budget, no radius): the first snapshot takes
  2.0 kB for 100 people, 20.6 kB for 1000, 212 kB for 10 000 (about 21 bytes each); after it
  0.34, 3.4 and 33.7 kB at 30 Hz, the quarter standing still costing nothing.
- **CPU, one thread** (the dev machine, other sessions running): the world, 3000 people
  quantised and bucketed, 0.14–0.15 ms a snapshot tick; a client's snapshot 37–43 µs to
  encode, 19–22 µs to decode; an ack 1.4–1.6 µs. For 100 clients the server spends 4.2 ms a
  snapshot tick, 13 % of a core at 30 Hz (`docs/PROFILE.md`).

Over QUIC (`net-loopback --clients 100 --replicate 3000`, one thread for the server, its
transport and the 100 clients, 50 ms and 2 % loss each way):

| Run | Down per client | Up per client | Delivered | Decoded |
|---|---|---|---|---|
| 20 s | 22.5 kB/s UDP payload, **23.8 kB/s with IPv4**, 48 datagrams/s | 5.9 kB/s, 7.5 with IPv4 | 98.0 % | all; 479 people held a client |
| 10 min | 22.4 kB/s UDP payload, **23.6 kB/s with IPv4**, 44 datagrams/s | 5.8 kB/s, 7.2 with IPv4 | 98.0 % (1 763 864 of 1 800 000) | all; 480 people held a client |

D-010's second proof stage was 100 bots at 24 kB/s or less over 100 ms and 2 % loss for
10 minutes; the 10-minute run meets it on loopback, on one machine, with the crowd standing in
for the population of #250. Its round trip read 155–188 ms (124 ms in the 20 s run): one thread
carried the server, 100 clients' QUIC and 6 ms of replication a tick (0.47 ms for the crowd and
its world, 57 µs a client), beside other sessions' builds on the same machine, so packets waited
for it. The server PC with the clients on other machines is the real test.

## Lag compensation (step 3's first piece, 2026-10-23)

`forge_sim::rewind`: the authority keeps each tick's hit shapes for a window, and tests a shot
against the world as its shooter saw it (server-side rewind, Bernier 2001).
- **`History`:** each tick's capsules by entity id, the oldest dropped past the window (12 ticks at
  60 Hz, 200 ms, about what shooters allow).
- **A shot carries its `View`:** the tick and the fraction of the next at which its shooter was
  drawing the others. The game puts it in its own command; the client knows it from its
  interpolation.
- **`History::ray`:** the two ticks about the view interpolated, the shot tested against their
  capsules (`Capsule::ray`, side and ends, from inside too), the shooter left out, the nearest hit
  taken, ties to the lower id. A view older than the window is clamped to its oldest tick, so an
  old claimed view gains nothing past it.
- **Patents:** searched again before building it (`docs/research/netcode.md` §9): none found on the
  rewind. Amazon's US 10,940,388 B1 and US 11,771,984 B2 claim hit volumes or shown places set from
  the measured variance of a client's input timing. Forge measures none: the view is the client's
  own, and its interpolation delay is fixed.
- **Tests:** a target running across at 6 m/s, drawn 100 ms behind by the shooter. Aimed where the
  shooter saw it, the shot hits at the shooter's view and misses at the authority's present tick.
  An old view is clamped. The capsule's ray is checked on its side, its ends, past it, past the
  reach and from inside.

### The client's picture and the moment a shot carries (2026-10-23)

`forge_net::interpolate`: what the client draws, and so what its shots claim.
- **`Interpolator`:** each entity's recent states by the server tick they were sent at, taken
  from the receiver after each packet it applied. An entity is drawn at a moment between its two
  states about it, or at its first or last outside them, never carried past what was sent. One
  the receiver no longer holds is forgotten.
- **`PlayoutClock`:** the moment to draw is the server's tick the freshest recent snapshot implies
  (its tick minus the local time it came, the largest over the last 30), minus a fixed delay. The
  delay covers the gaps between a near person's states (7 ticks, 117 ms, in the test: three
  snapshot intervals at 30 Hz and one tick of jitter).
- **A shot carries that `Moment`**, a tick and a fraction, which the server takes as its rewind's
  `View`.
- **Patents:** the delay is the client's own and never set from how its packets arrive. The
  offset follows the freshest arrival, which is clock synchronisation, not a measure of the
  timing's variance that Amazon's two patents claim (§9).

**End to end** (`tests/lag_compensation.rs`, on a virtual clock):
- **The setup:** 400 people in a 120 m square, replicated at 30 Hz over the bad link both ways
  (50 ms, 10 ms of jitter, 5 % lost). From 2 s in, every 6 ticks the client shoots at the chest
  of the nearest person it draws 5–30 m away, the shots over a bad link of their own. The server
  tests each as it arrives against its hit history (60 ticks of capsules) at the shot's moment.
- **The result:** of 177 shots that arrived, 176 meet the person the client saw first along the
  ray. Their distance is off from the client's by 1.7 mm at the median and 5.0 mm at most (the
  1 cm quantisation and the interpolation). The one miss grazed a nearer person: the ray passes
  0.3025 m from that person's axis, outside the 0.30 m radius by 2.5 mm.
- **Without the rewind,** at the server's present tick, 129 of the 177 meet that person: the
  people have moved on since the moment the client drew, about 0.2 s before (the 117 ms delay
  and the two 50 ms links): a walker at 1.4 m/s by some 0.3 m.
- **Deterministic:** two runs give the same shots and the same errors.
- **Eight clients** (D-010's hit-registration stage, in one process): eight clients spread 30 m
  round the square's middle, over links of 10, 25, 40, 55, 70, 90, 110 and 125 ms each way (round
  trips of 20 to 250 ms, a fifth of the delay as jitter, 5 % lost), each shooting at the people it
  draws. Every shot that arrived met the person its client saw, 168 to 179 a client, the distance
  off by 4.2 to 6.1 mm at the 99th percentile. At the server's present, 91 to 173 a client would.

## Pacing the client (step 3, 2026-10-23)

`forge_sim`'s clients (#137) ran ahead of the server by a lead set at the start from the link's
delay. D-010 paces them instead:
- **The measure:** every input packet carries its client's tick (`InputPacket::tick`), and a
  packet goes every tick, commands or not. The server takes each one's lead on arrival (the
  client's tick less its own) and reports the least of the last 60 in each snapshot
  (`Snapshot::leads`): a measure, never a pace.
- **The clock:** `PaceClock` runs the client's ticks from the time that passes at a rate that
  follows the lead's error alone, 2 % a tick of lead missed, within ±10 %: faster when its packets
  arrive with less lead than its target, slower with more. Only the simulation's clock is paced.
- **A late command waits:** the server takes it at its next tick and the client is corrected, as
  before. Nothing is repeated, guessed or extrapolated.
- **Patents** (`docs/research/netcode.md` §9, searched first): no claim was found on pacing by
  the lead's measure. Google's US 9,479,618 B2 claims a fixed ring of the last commands resent in
  each packet; Forge's client resends what is not yet acknowledged and drops a command only on
  its acknowledgement.

**The test** (`a_paced_client_keeps_its_lead_as_the_route_changes`): one client over a link of
100 ms one way, starting with no lead and aiming at 3 ticks; at 20 s the route goes to 200 ms, at
40 s to 50 ms.
- From no lead it reaches 3 ticks in about 5 s (it overshoots to 5), and stays at 3.
- When the delay doubles, the lead falls to −3 and comes back to 3 in about 6 s. When it falls to
  a quarter, the lead jumps to 7 and comes back in about 4 s.
- 10 of the 300 commands came late, all in those changes, none once settled; every command was
  taken once.

**Corrections faded on screen** (the physics lab's `--net`, `docs/demos/physics-lab.md`): a
corrected body stays drawn where the prediction had it and the difference fades to 5 % in 0.1 s,
by a fixed time, never chosen from the error or the motion (Tencent's US 12,157,057 B2 claims a
correction chosen that way); one more than 2 m off is drawn at once. At 100 ms one way the lab's
corrections moved a body by 0.31 m (the sled) to 0.47 m (the pyramid hit by the bot's throws).

## Replays (step 3, 2026-10-23)

The research's recommendation (`docs/research/netcode.md`): record every input with its tick and
the whole state every 10 s; a replay must reproduce the run to the bit.
- **The server records what it applies** (`Server::record`, `forge_sim::Recorder`): every
  player's commands at the tick it applied them (a late one at the tick it took it), a digest
  every 60 ticks, and its whole state every `keyframe_every` ticks, the first at the start.
- **A replay starts anywhere** (`Recording::seek`): the last keyframe at or before the tick is
  restored and the recorded ticks between are run, their digests checked. `Recording::replay`
  runs from the start, as before. The file format is `FRGREC02`: the keyframes after the
  digests, each packed with LZ4 as the caches are (`forge_core::pack`). `FRGREC01` files still
  read.
- **Watching and checking only:** nothing lets play go on from a recorded moment. Blizzard's
  US 9,089,777 family (to US 11,654,356, about 2033-04) claims resuming gameplay from a point
  between checkpoints by loading the saved states and replaying the inputs; a replay that only
  reproduces the past stays outside it (`docs/research/netcode.md` §9).
- **The tests:** `forge-sim`'s two players over 100 ms and 2 % loss, the server's recording
  replayed on a fresh world (every digest and the server's world at the end), started at ticks
  0, 299, 300, 777 and the end (the same world as from the start), its bytes round-tripped, a
  changed command caught at the next digest from the keyframe before it. The lab's tug-of-war the
  same way (`a_net_session_recorded_on_the_server_replays_and_starts_anywhere`).
- **In the lab** (`physics-lab --lab drop --net 100 --throw-every 45 --record FILE`, 1500
  ticks): 42 commands of both players (one late), 25 digests, 3 keyframes, 464 KiB (648 KiB
  unpacked, nearly all of it the three whole states). `--replay FILE` checks the 25 digests;
  `--replay FILE --seek 900` restores the keyframe at 600 and runs 300 ticks in 0.18–0.22 s before
  the first frame, then checks the last 10. At tick 1200 both draw every body in the same place.

## The simulation over QUIC, through the network thread (2026-10-23)

Until now `forge-sim`'s scheme (stamped commands sent again until acknowledged, snapshots checked
by digest, the paced clock) ran over in-process links only, and the transport's tests were
async. `forge_net::NetThread` puts the tokio runtime on a thread of its own, `forge-net`: the
simulation's thread connects through it (`connect`, waiting for the handshake at start-up) or
serves through it (`serve`: `Listener::try_accept` hands over each new client's session, never
waiting); then it sends and takes messages on plain sessions, `send` and `try_recv` never
waiting either. `InputPacket` and `Snapshot` have their bytes (`Codec`).

**The test** (`tests/sim_over_quic.rs`): a server and two clients in synchronous loops, the
clock four times fast (240 ticks a second), each link 40 ms one way with 4 ms of jitter and 2 %
lost, through the conditioner both ways; 20 pushes each over 900 ticks. Four runs:

| | Client 0 | Client 1 |
|---|---|---|
| Snapshots taken (of 200) | 194–197 | 192–197 |
| Predicted to the bit / corrected | 176–179 / 18 | 170–175 / 21–22 |
| Lead on arrival once settled (target 4) | 2–5 ticks | 2–5 ticks |

Every command was taken once and none late, and each client ended where the server is.

**The physics lab over QUIC** (`physics-lab --lab tug --net 100 --net-quic`): the lab's server
and its two players in the app's own frame loop, the players' packets on the unreliable channel
and the snapshots (whole states) on the reliable ordered one, 100 ms each way and 2 % lost
through the conditioner, on the wall clock. 6000 frames of the tug-of-war, the in-process links
against QUIC: 152 against 135 snapshots, 10 against 8 corrections, every command taken and none
late; a correction runs more ticks again over QUIC (673 in all against 140, at most 6.2 ms),
since a snapshot held back for a resent packet arrives older. **`drop` does not keep up:** its
whole states, 280 KB ten times a second, need 2.8 MB/s, and QUIC's congestion control over a
200 ms round trip losing 2 % gives about 50 kB/s (1.22 · MSS / (RTT · √p)): one snapshot in a
run. The replication's deltas, quantisation and budget above are what a real scene goes through.

## Authority between workers (step 2's handoff, 2026-10-23)

`forge_sim::authority`: D-010's replication layer and its workers, in one process, before any
socket. The layer holds every entity's state, the worker that owns it and an epoch; a worker
holds nothing between ticks.
- **The regions:** `Strips`, the world cut along x, one strip a worker (D-010's first stage:
  static regions; splitting and merging them by load comes later). An entity stays with its
  owner until it is `margin` past the owner's border, so one walking along a border does not
  change hands every tick.
- **The layer:** `Layer::write` takes a worker's write only when it carries the record's epoch
  and comes from its owner: a fencing token, so a worker that has not heard that authority
  moved on writes in vain. `Layer::rebalance`, at the tick's end, hands every entity past its
  owner's border by the margin to the owner of the region it stands in, its epoch one more.
  The handoff takes effect at the next tick's barrier: it costs no tick.
- **The workers:** at each tick's barrier, `Worker::sync` takes from the layer the entities the
  worker owns, with their epochs, and as read-only ghosts the others within `band` of its
  region; `Worker::step` steps its own from that picture. A worker killed and started again
  takes the same picture. A worker gone is covered: `Layer::reassign` gives its region to
  another, and back when it returns.
- **Why the split world steps as one:** a step reads the world as the last tick left it (an
  entity from its own state and its neighbours' at the barrier), the conservative scheme of
  distributed simulation with one tick of lookahead (Fujimoto). With a band as wide as the
  margin and the reach of what an entity reads, a worker's picture holds everything its
  entities read.
- **Patents** (`docs/research/netcode.md` §9, searched before building it): Improbable's handover
  patent (US 10,878,146 B2) claims telling the old worker in advance that authority will move
  after a period; here it moves at a tick's end with no notice. Authority is whole-entity and
  each worker takes one picture of the layer a tick, not the add, remove and delegate messages per
  component of US 10,289,767 B2. Interactions across a border are read from the ghosts, and the
  layer and the workers share one tick. Moving regions by load later (Amazon's and Improbable's
  balancing patents) needs its own search.

**The test** (`crates/forge-sim/tests/authority.rs`): 200 bots in a 24 × 20 m strip patrol
back and forth across x = 0, each pushed off the neighbours within 1.5 m. Two workers own the
two sides, an entity changing hands 2 m past the border; each sees 3.51 m beyond its side.
Over 1800 ticks (30 s), against one worker holding everything:

| Run | Handoffs | Writes refused | Records against one worker |
|---|---|---|---|
| two workers | 292 | 0 | the same, every tick, to the bit |
| revokes reaching the old owner up to two ticks late (it keeps writing at its old epoch) | 292 | 312 | the same |
| worker 1 away from tick 400 to 600, worker 0 covering its region | 244, and 192 reassigned | 0 | the same |


**Over the transport** (`forge_net::layer`): the layer on a server, each worker a client, in
lockstep on the first channel (reliable, ordered).
- **Each tick:** the layer sends each worker one picture (its own entities with their epochs, its
  ghosts); each worker steps its own and sends its writes back (`layer::serve`); once every worker
  has answered, the layer takes the writes and hands over what crossed.
- **A worker gone:** a session that closes, or stays silent past the host's patience, is covered
  by the lowest worker left. The tick's writes are set aside and the tick is stepped again from the
  same pictures, so no tick is lost. A message carries its frame, so an answer to the first try is
  dropped. A worker that comes back is attached again and takes its region back.
- **The test** (`tests/layer.rs`, QUIC on loopback): the same 200 bots for 900 ticks, against one
  worker holding everything in one process: 142 handoffs, the records the same every tick to the
  bit. Worker 1 killed at tick 400 before it answers (its session closed): covered by worker 0 on
  that tick, 192 entities reassigned there and back when it returns at tick 600, the records still
  the same. The 900 ticks take 0.3 s.
- **Under the clients** (`net-loopback --replicate 3000 --workers N`): the crowd that the 100
  clients receive is stepped at 60 Hz by N workers through the layer, each owning a strip of the
  square; the server reads the records for its snapshots. With 50 ms and 2 % loss to the clients,
  20 s, the clients' numbers don't move (22.5 kB/s down, 98.05 % delivered, every snapshot
  decoded). The layer's tick, its pictures out and its workers' writes back over QUIC on
  loopback, all on the server's one thread:

  | People | Workers | Handoffs in 20 s | The layer's tick |
  |---|---|---|---|
  | 300 | 2 | 1 | 0.53 ms |
  | 1000 | 2 | 6 | 1.03 ms |
  | 3000 | 2 | 48 | 3.55 ms |
  | 3000 | 4 | 147 | 3.92 ms |

  About 0.3 ms a tick plus 1.1 µs a person, against 0.15 ms to step the 3000 in place: every
  picture carries each entity's whole state and every ghost each tick. Pictures by change, and
  ghosts only within the band, are the first savings when the layer meets real load.

## Next

- The layer and its workers on separate machines (the server PC), and a worker's region split
  and merged by load (searched 2026-10-23: re-cut on a worker joining or leaving first, cuts moved during play on the owner's word, `docs/research/netcode.md` §9); the skinned parts' corrections faded too;
  lag compensation in a scene (a shot in the lab's `--net`, the client's `Interpolator` drawing
  what it aims at); the layer's own replay (its workers' writes recorded, diffed against a fresh
  run, as the research's stage 4 asks); and the `hundred-bots` demo on the server PC.
- The population of #250 as the replicated world: its near tiers' records in place of the crowd.
- The physics lab's scenes replicated rather than sent whole, so `drop` runs over QUIC too (its
  bodies as the replicator's entities, the rest of the state rebuilt on the client).
- The server PC and the Raspberry Pi come later, on the owner's word: nothing here binds
  anything but loopback.
