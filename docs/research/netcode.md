# Netcode for large shared worlds

An annotated bibliography for Forge's network layer: the shooter-grade canon (snapshots, deltas,
prediction, lag compensation, rollback), the transport question in 2026 (raw UDP with a custom
reliability layer versus QUIC with unreliable datagrams, and WebTransport for browsers), replication
and bandwidth for worlds with thousands of entities, determinism, the server side of a living world
(workers, replication layer, persistence, message bus) and security. Organised by problem, not by
date, in the same shape as [RESEARCH.md](../RESEARCH.md): every entry is tagged with its kind —
**[paper]**, **[book]**, **[talk]**, **[web]**, **[code]**, **[spec]** — and with how it has aged
(**foundational**, **still-current**, **recent**), and ends with a *Bearing* line for Forge. The
topology it argues for is the one in the world SPEC §10.2–10.5: a replication layer holding hot
entity state, nearly stateless simulation workers, RabbitMQ only for asynchronous events, MongoDB for
persistence. Two in-house lessons are taken as given: a starved input queue must *wait*, never
replay the last input; and a reconciliation that reports hundreds of corrections of 0.000 m is a
threshold bug, not a network condition.

> **State of the art in five sentences.** Shipped shooters still run the Quake/Source model — an
> authoritative server at 60–128 Hz, delta-compressed snapshots against the last acknowledged
> baseline, client prediction with rollback of the predicted state, ~100 ms interpolation for
> everyone else, and server-side rewind for hit registration — and the best public description of the
> whole thing is still Fiedler's networked-physics series plus Overwatch's GDC 2017 talk. Transport has
> moved: QUIC (RFC 9000) with unreliable datagrams (RFC 9221) gives encrypted, authenticated,
> congestion-controlled packets with no head-of-line blocking across streams, and since Safari 26.4
> (March 2026) WebTransport reaches every major browser, so one QUIC-based protocol can serve native and
> web clients. Bandwidth is won by quantisation and bit-packing (smallest-three quaternions,
> cell-relative positions), by a per-client priority accumulator that spends a fixed byte budget on the
> most stale-and-important entities, and by interest management that stops the server considering most
> of the world for most clients (Fortnite's Replication Graph: 100 players, ~50,000 replicated actors).
> Very large single-shard worlds are real but rare: EVE degrades gracefully with time dilation, Star
> Citizen's replication layer plus static server meshing went live in Alpha 4.0 (December 2024) after
> years of work, and SpatialOS — the generic answer — lost every game built on it. In Rust the
> transport and serialisation layers are mature (`quinn`, `rustls`, `bitcode`), the gameplay-level
> crates (`lightyear`, `bevy_replicon`, `naia`) are usable but Bevy-coupled and none implements the
> priority/delta/interest triad a large world needs, so a "pro" engine owns its replication protocol
> and takes only transport, crypto and ECS from crates.

**Contents**

1. [The canon](#1-the-canon)
2. [Transport in 2026](#2-transport-in-2026)
3. [Replication, bandwidth and big worlds](#3-replication-bandwidth-and-big-worlds)
4. [Prediction, reconciliation and lag compensation](#4-prediction-reconciliation-and-lag-compensation)
5. [Determinism and the simulation](#5-determinism-and-the-simulation)
6. [Server architecture for a living world](#6-server-architecture-for-a-living-world)
7. [Security and crypto](#7-security-and-crypto)
8. [Voice](#8-voice)
9. [Recommendation for Forge](#recommendation-for-forge)
10. [Checked and left out](#checked-and-left-out)
11. [Verification notes](#verification-notes)

---

## 1. The canon

Read in this order: Fiedler for vocabulary and numbers, Sanglard and the Valve wiki for the model
every shooter still uses, Bernier for why, the GDC talks (Halo: Reach, Overwatch, Rocket League,
Destiny, NetherRealm) for what changes when a real game ships it, and Tribes for the oldest statement
of "partial updates by priority", which is the model a large world needs.

### Fiedler's networked-physics series and libraries

**Glenn Fiedler. "Introduction to Networked Physics" (28 Nov 2014) and "Networking for Physics
Programmers" (GDC 2015, Physics for Game Programmers tutorial).** [web] [talk] [foundational]
<https://gafferongames.com/post/introduction_to_networked_physics/> ·
<https://www.gdcvault.com/play/1022195/Physics-for-Game-Programmers-Networking>

Opens the series the rest of this section cites: one physics demo (a player cube pushing hundreds of
cubes, ending in a Katamari-style coupled pile) networked three ways — deterministic lockstep,
snapshot interpolation, state synchronisation — so trade-offs are measured on one scene rather than
argued. The GDC talk is the one-hour version.
*Bearing:* the three strategies are the columns of every decision below; Forge's model is "state
synchronisation for what the client predicts, snapshot interpolation for everything else".

**Glenn Fiedler. "Snapshot Interpolation." Gaffer On Games, 30 Nov 2014.** [web] [foundational]
<https://gafferongames.com/post/snapshot_interpolation/>

Send the visual state of everything, render it in the past, interpolate (Hermite for position, slerp
for orientation). The numbers: 60 snapshots/s cost 11.6 Mbit/s for the cube scene; 10/s cost 2 Mbit/s
but force a 350 ms buffer; 30/s allow ~150 ms and 60/s ~85 ms. The buffer is roughly two send
intervals plus jitter — the origin of the industry's "100 ms at 20–30 Hz".
*Bearing:* Forge's 30 Hz snapshots with a 100 ms buffer sit on this curve; a 20 Hz tier for distant
entities means ~150 ms for them, which is fine for things you cannot touch.

**Glenn Fiedler. "Snapshot Compression." Gaffer On Games, 4 Jan 2015.** [web] [foundational]
<https://gafferongames.com/post/snapshot_compression/>

The bandwidth recipe with measurements: 900 cubes at 60 Hz start at 17.38 Mbit/s; smallest-three
quaternions (128 → 29 bits), bounded quantised positions (96 → 50 bits), an at-rest flag, then delta
encoding against the last snapshot the receiver *acknowledged* bring it to about 256 kbit/s — a 70×
reduction with no visible loss. The sender must keep per-client baseline history and fall back to a
full state when the ack is too old.
*Bearing:* the implementation list for Forge's "deltas and quantisation" stage, with its expected
order of magnitude: from ~40 bytes per entity per tick raw to well under one byte on average.

**Glenn Fiedler. "State Synchronization." Gaffer On Games, 5 Jan 2015.** [web] [foundational]
<https://gafferongames.com/post/state_synchronization/>

Send inputs *and* state, let the receiver extrapolate with its own physics, stop requiring
determinism. Two ideas Forge needs: the **priority accumulator** (each object's priority accrues every
frame; the packet is filled with the highest accumulated values, which reset on send) so a fixed
budget always goes to the most important stale objects, and **quantise on both sides** so the sender
simulates the same rounded values the receiver sees. Target 256 kbit/s for the same scene, with delta
compression "another order of magnitude" on top.
*Bearing:* the per-client priority accumulator is what makes a large world affordable — nothing is
"sent every tick", everything is sent when its accumulated importance wins the budget.

**Glenn Fiedler (mas-bandwidth). `netcode`, `reliable`, `yojimbo`; "Reliable Ordered Messages"
(15 Sep 2016).** [code] [web] [still-current]
<https://github.com/mas-bandwidth/netcode> · <https://github.com/mas-bandwidth/reliable> ·
<https://github.com/mas-bandwidth/yojimbo> · <https://gafferongames.com/post/reliable_ordered_messages/>

The reference "pro raw UDP" stack, BSD-3 C. `netcode` (standard 1.02) is a secure client/server
connection protocol: a web backend issues a **connect token** whose private part is encrypted with a
key shared with the servers and carries client id, expiry, server list and two session keys; packets
are then encrypted per direction. `reliable` adds acks, fragmentation and RTT/loss estimates on the
design in the article: a sequence number plus a 32-bit ack bitfield per packet, *messages* re-included
in packets until a containing packet is acked, sequence buffers indexed `seq % N`. `yojimbo` adds
channels, bit-packing and a network simulator. Rust port: `renetcode` (§2).
*Bearing:* the connect-token model — authenticate on the web, hand out a short-lived token, let the
server verify it offline — is how Forge should attach a web login to a QUIC connection; the ack
bitfield design is what to build if Forge ever needs reliability over datagrams.

### The model every shooter still uses

**Fabien Sanglard. "Quake 3 Source Code Review: Network Model." fabiensanglard.net, 2012.** [web]
[foundational]
<https://fabiensanglard.net/quake3/network.php>

UDP only; the server keeps the last 32 snapshots per client and sends a *delta* against the last
snapshot the client acknowledged, so a lost packet only makes the next delta larger; messages are
Huffman-compressed and pre-fragmented into 1400-byte chunks; reliable commands ride a small reliable
layer inside the same NetChannel.
*Bearing:* the per-client ring of acknowledged baselines is the data structure Forge's replication
layer needs per (client, entity); 32 baselines at 30 Hz is one second of history, which is enough.

**Valve. "Source Multiplayer Networking." Valve Developer Community wiki.** [web] [foundational]
<https://developer.valvesoftware.com/wiki/Source_Multiplayer_Networking>

The numbers a generation copied: 66.67 Hz ticks (15 ms); ~20 snapshots/s (`cl_updaterate 20`) and
~30 command packets/s bundling several ticks of input; deltas against the last acknowledged update
with a full snapshot only after heavy loss; 100 ms interpolation (`cl_interp 0.1`) so one lost
snapshot still leaves two to interpolate between; lag compensation rewinds the server to what the
shooter saw. Worst-case modem clients took 5–7 KB/s — the origin of the per-client `rate` budget.
*Bearing:* Forge's "30 Hz snapshots, 100 ms interpolation, per-client byte budget" is Source's design
with a faster snapshot rate; this is the citation for why 100 ms and not less.

**Yahn W. Bernier. "Latency Compensating Methods in Client/Server In-game Protocol Design and
Optimization." GDC 2001. — Gabriel Gambetta, "Fast-Paced Multiplayer" (Parts I–IV).** [paper] [web]
[foundational]
<https://www.gamedevs.org/uploads/latency-compensation-in-client-server-protocols.pdf> (listed with
citation line in Claypool's taxonomy, §4) · <https://www.gabrielgambetta.com/client-server-game-architecture.html>

The Half-Life paper that introduced the trio: client-side prediction by re-running the same movement
code, reconciliation by replaying unacknowledged commands on top of the server's last state, and
server-side **lag compensation** — rewinding other players' hit volumes to the time the shooter saw
them. Gambetta's four pages with interactive demos are the teaching version of the same material.
*Bearing:* the design Forge's M1 walkers already follow; lag compensation is the missing piece and
needs a per-entity history ring on the authoritative worker.

**Mark Frohnmayer, Tim Gift. "The TRIBES Engine Networking Model." Dynamix, 1999 (widely cited as
2001).** [paper] [foundational]
<https://archive.org/details/tribes-networking-model> ·
<https://www.gamedevs.org/uploads/tribes-networking-model.pdf>

The oldest clear statement of eventual-consistency replication for a large world: a layered stack
(packet → connection → stream managers) with a **ghost manager** that keeps per-client *scoped* object
sets, sends *partial* object state by priority within a packet budget, and a delivery-notification
protocol so the sender knows which states arrived. Designed for 32–128 players on 1998 modems.
*Bearing:* Tribes' ghost/scope/priority triad is the shape of Forge's replication layer.

### What shipping it looks like

**David Aldridge. "I Shot You First: Networking the Gameplay of Halo: Reach." GDC 2011.** [talk]
[foundational]
<https://www.gdcvault.com/play/1014345/I-Shot-You-First-Networking>

Bungie's host-authoritative model with client prediction: replication budgeted per client and driven
by per-object priority functions, different delivery rules for state, events and player controls,
and the rule in the title — the shooter sees their shot land immediately and the host arbitrates
afterwards. Fiedler's priority accumulator is the same idea named.
*Bearing:* the source for "priority is a function of distance, size, importance and staleness,
evaluated per client", and for showing predicted effects at once and undoing them rarely.

**Timothy Ford. "'Overwatch' Gameplay Architecture and Netcode." GDC 2017.** [talk] [still-current]
<https://www.gdcvault.com/play/1024001/-Overwatch-Gameplay-Architecture-and>

ECS on client and server, a fixed 16 ms tick, and the cleanest public description of prediction with
**rollback of predicted state**: the client predicts movement and abilities, the server confirms
inputs by tick, and on mismatch the client restores the last confirmed state and re-simulates its
buffered inputs. The server keeps a small input buffer per client and, when loss starves it, tells
the client to speed up — the client paces itself to the server. Hit registration favours the shooter
with bounded rewind.
*Bearing:* the design Forge's "clients pacing their inputs to the server's queue" item should copy,
and the argument for a rollback-able store of predicted components separate from the rest of the ECS.

**Jared Cone. "It IS Rocket Science! The Physics of 'Rocket League' Detailed." GDC 2018.** [talk]
[still-current]
<https://www.gdcvault.com/play/1024972/It-IS-Rocket-Science-The>

Server-authoritative physics where the client predicts *everything it can touch* — its car, the ball,
other cars — with a fixed-step engine, re-simulates from the server's state when a correction
arrives, and smooths rather than snaps. The existence proof that physics-driven objects can be
predicted and reconciled without determinism.
*Bearing:* Forge's ships and walkers touching physics objects (docking, pushing crates) should do the
same: predict the contact group, roll back the group, smooth.

**Justin Truman. "Shared World Shooter: Destiny's Networked Mission Architecture." GDC 2015.**
[talk] [still-current]
<https://www.gdcvault.com/play/1022247/Shared-World-Shooter-Destiny-s>

A hybrid: server-hosted mission simulation plus peer-to-peer player state, the world split into
spatial areas with their own hosts, host migration and disconnect handling — and the talk is honest
that each of these leaked into mission design.
*Bearing:* authority boundaries designers can feel become design constraints; Forge's rule that a
handoff costs at most one tick is the right target.

**Michael Stallone. "8 Frames in 16ms: Rollback Networking in 'Mortal Kombat' and 'Injustice 2'."
GDC 2018. — Tony Cannon, `ggpo` (MIT).** [talk] [code] [still-current]
<https://www.gdcvault.com/play/1025471/8-Frames-in-16ms-Rollback> · <https://github.com/pond3r/ggpo>

Rollback for a deterministic fighting game: simulate without the remote input, then restore a saved
state and re-simulate up to eight 60 Hz frames inside one 16 ms frame. The engine work — fast state
save/restore, simulation separated from rendering, deferred audio/VFX — is what state-sync prediction
needs too. GGPO is the SDK that popularised the technique (save/load/advance callbacks).
*Bearing:* "N frames of resimulation per frame" sizes the client's predicted-state store and physics
step cost; GGPO's three callbacks are the minimal interface a rollback-able ECS world must expose.

**Jay Mattis. "Netcode Architectures" Parts 1–4 (Lockstep, Rollback, Snapshot Interpolation,
Tribes). SnapNet blog, 2023.** [web] [recent]
<http://www.snapnet.dev/blog/netcode-architectures-part-3-snapshot-interpolation/> ·
<http://www.snapnet.dev/blog/netcode-architectures-part-4-tribes/>

A practitioner's comparison using shipped games (Quake III, Counter-Strike, Overwatch, Apex, Call of
Duty). The useful part is the scaling argument: snapshot interpolation's cost grows with world size
because every client's packet considers every entity, while Tribes-style partial, prioritised updates
make bandwidth and CPU proportional to what each client can see.
*Bearing:* the argument for building Forge as Tribes-plus-prediction rather than
Source-plus-a-bigger-map, and for a client designed defensively against partial state.

---

## 2. Transport in 2026

### The specifications

**Jana Iyengar, Martin Thomson (eds). "QUIC: A UDP-Based Multiplexed and Secure Transport." RFC
9000, IETF, May 2021.** [spec] [still-current]
<https://www.rfc-editor.org/rfc/rfc9000.html>

Encrypted transport over UDP: independent flow-controlled streams (loss on one does not block
another), connection IDs that survive address changes, 0-RTT resumption, and a 3× anti-amplification
limit before address validation with a 1200-byte minimum initial datagram — built-in defence against
reflection attacks.
*Bearing:* the head-of-line problem WebSockets gave the first in-house project is solved at the
protocol level; per-stream ordering is what Forge's reliable event channels use.

**Tommy Pauly, Eric Kinnear, David Schinazi. "An Unreliable Datagram Extension to QUIC." RFC 9221,
March 2022. — Gorry Fairhurst et al., "Packetization Layer Path MTU Discovery for Datagram
Transports." RFC 8899, September 2020.** [spec] [still-current]
<https://www.rfc-editor.org/rfc/rfc9221.html> · <https://www.rfc-editor.org/rfc/rfc8899.html>

DATAGRAM frames: not retransmitted, still ack-eliciting (the sender learns of loss), congestion
controlled, sharing the connection's crypto — and *not fragmentable*, so bounded by
`max_datagram_frame_size` and the path MTU. RFC 8899 is how a datagram transport probes for a larger
MTU without ICMP (`s2n-quic` implements it; `quinn` has MTU discovery too); without it, ~1200 bytes is
the safe ceiling.
*Bearing:* inputs and snapshots go in datagrams; a snapshot is "as many priority-ordered entity blocks
as fit in one datagram", never a fragmented blob. Budget for 1200 bytes and treat PMTUD gains as bonus.

**W3C. "WebTransport." Candidate Recommendation Snapshot, 30 July 2026 (eds. Jaju, Vasiliev,
Bruaroey); IETF `draft-ietf-webtrans-http3-16`, 6 July 2026 (WG Last Call). — Glenn Fiedler, "Why
can't I send UDP packets from a browser?" (26 Feb 2017).** [spec] [web] [recent]
<https://www.w3.org/TR/webtransport/> · <https://datatracker.ietf.org/doc/draft-ietf-webtrans-http3/>
· <https://webrtc.ventures/2026/04/webtransport-is-now-baseline-what-it-means-for-real-time-media/>
· <https://gafferongames.com/post/why_cant_i_send_udp_packets_from_a_browser/>

Browser API for streams *and unreliable datagrams* over HTTP/3, i.e. QUIC. Chrome 97 (2022) and
Firefox 114 (2023) shipped it; Safari 26.4 (March 2026) made it Baseline; the IETF binding is in
Working Group Last Call, so the wire format is effectively frozen. Fiedler's 2017 post is the
complaint it answers: WebRTC is a peer-to-peer stack forced onto client/server games.
*Bearing:* a browser client for Forge is a WebTransport server (`wtransport`, on `quinn`) in front of
the same protocol — one codebase for native and web, which no raw-UDP design can offer; do not build
on WebRTC data channels for client/server play.

### Libraries outside Rust

**Lee Salzman. `ENet`. MIT, 2002–2026.** [code] [foundational]
<https://github.com/lsalzman/enet>

Reliable/unreliable/sequenced packets, channels, fragmentation, throttling; no encryption, no
authentication, no real congestion control. What a great many indie games shipped on.
*Bearing:* the baseline to beat; Forge's channels must at least match ENet's semantics.

**Valve. `GameNetworkingSockets` (BSD-3) and Steam Datagram Relay; Epic Games, Epic Online
Services.** [code] [web] [still-current]
<https://github.com/ValveSoftware/GameNetworkingSockets> ·
<https://partner.steamgames.com/doc/features/multiplayer/steamdatagramrelay> ·
<https://dev.epicgames.com/docs/epic-online-services>

Valve's open-source transport: reliable and unreliable messages, fragmentation, an ack-vector model
from DCCP/QUIC, AES-GCM-256 packets with Curve25519 key exchange, ICE-based P2P, prioritised "lanes",
a built-in lag/loss simulator. Linked against Steamworks it gains **SDR**, Valve's relay backbone that
hides server and client IPs (DDoS protection) and often lowers latency; it needs a Steam presence and
ticket-authenticated servers. EOS offers free cross-platform accounts, lobbies/matchmaking, P2P with
relay, voice and anti-cheat.
*Bearing:* GNS is the design most like a hand-built QUIC-class protocol; SDR is the cheapest credible
answer to a launch-day DDoS once Forge is on Steam; EOS is a candidate for matchmaking, voice and
anti-cheat around Forge's own world protocol, not a transport for it.

### Rust crates

**Dirkjan Ochtman, Benjamin Saunders et al. `quinn`. MIT/Apache, 0.11.12 (Sep 2026); AWS,
`s2n-quic`, Apache-2.0, 1.89.0 (Sep 2026).** [code] [still-current]
<https://github.com/quinn-rs/quinn> · <https://github.com/aws/s2n-quic>

The pure-Rust QUIC most of the ecosystem uses (~39M downloads/month): tokio API, `rustls` crypto,
RFC 9221 datagrams, 0-RTT, migration, and `quinn-proto`, a deterministic sans-I/O state machine you
can drive from a simulator. `s2n-quic` is the alternative with CUBIC, pacing, GSO and RFC 8899 PMTUD;
Windows supported with `rustls`.
*Bearing:* keep `quinn`; `quinn-proto` is how to unit-test the protocol under scripted loss without
sockets.

**Lucas Poffo. `renet` / `renetcode`. MIT/Apache, 2.0.0 (Jan 2026).** [code] [still-current]
<https://github.com/lucaspoffo/renet>

Channels (reliable-ordered, reliable-unordered, unreliable) over a pluggable transport; `renetcode`
implements netcode 1.02 (connect tokens, ChaCha20-Poly1305 packets); `renet_steam`, `bevy_renet` and
an egui metrics visualiser. No replication, no prediction — the Rust `yojimbo`.
*Bearing:* the fallback if QUIC's per-packet overhead ever matters, and the reference Rust
connect-token implementation.

**Charles Bournhonesque. `lightyear`. MIT/Apache, 0.30.1 (Sep 2026), Bevy 0.19.** [code] [recent]
<https://github.com/cBournhonesque/lightyear>

The most complete Bevy netcode crate: server-authoritative replication, prediction with rollback,
snapshot interpolation, lag compensation between predicted and interpolated entities, per-tick input
buffering with loss protection, a bandwidth cap with priority ordering, `postcard` by default, and
UDP / WebTransport / WebSocket / Steam transports.
*Bearing:* the best Rust code to *read* for prediction and lag-compensation plumbing; not Forge's
core because it is Bevy-coupled and assumes one authoritative server, not a replication layer with
moving authority.

**Project Harmonia / simgine. `bevy_replicon`. MIT/Apache, 0.44.2 (Sep 2026), Bevy 0.19.** [code]
[recent]
<https://github.com/projectharmonia/bevy_replicon>

Replication driven by Bevy change detection (not acked baselines), per-client visibility filters,
"mutate messages" tagged with server ticks so clients know which tick is fully confirmed, and
backends for `renet`, `renet2`, `bevy_quinnet`, `aeronet`, `matchbox`. Prediction is out of scope (a
sibling, `bevy_rewind`, offers Rocket-League-style rollback).
*Bearing:* the tick-confirmation bookkeeping is a good model; change-detection-only replication
without baselines, quantisation or a priority budget is exactly what Forge must go beyond.

**naia-lib. `naia`. MIT/Apache, 0.25 (May 2026).** [code] [still-current]
<https://github.com/naia-lib/naia>

ECS-agnostic entity replication with change detection and **delta compression**, rooms for scope,
tick-buffered input channels, authority delegation to clients, native UDP and browser WebRTC.
*Bearing:* the only Rust crate advertising delta compression and rooms together; read its
tick-buffered channel and delegation design.

**Johan Helsing, `matchbox` (0.14, Feb 2026); Georg Friedrich Schuppe, `ggrs` (0.13, Jun 2026).
MIT/Apache.** [code] [recent]
<https://github.com/johanhelsing/matchbox> · <https://github.com/gschup/ggrs>

WebRTC full-mesh data channels with a signalling server, paired with `ggrs`, a safe-Rust GGPO with
P2P, spectator and **sync-test** sessions (shipped titles on Steam).
*Bearing:* not for Forge's world, but the sync-test session — run two instances and diff state every
frame — is the determinism test Forge should copy for replays and the two-worker mode.

**Finnbear / Softbear Studios. `bitcode`. MIT/Apache.** [code] [recent]
<https://github.com/SoftbearStudios/bitcode>

Bit-level encoder that groups all instances of a field, uses as few bits as the type allows,
validates up front; optional `serde`; explicitly *not* stable across versions or self-describing.
*Bearing:* right for versioned, schema-known messages between Forge binaries from one tree (events,
RL↔worker traffic); wrong for the snapshot hot path, which needs quantised fields and per-client
deltas — that layer is a hand-written bit writer.

### Comparison of Rust networking crates (checked September 2026)

| Crate | Transport | Reliability | Encryption / auth | Delta compression | Interest mgmt | Prediction | ECS coupling | License | Activity 2025–26 |
|---|---|---|---|---|---|---|---|---|---|
| `quinn` 0.11.12 | QUIC (RFC 9000/9221) | streams + datagrams | TLS 1.3 (`rustls`) | — | — | — | none | MIT/Apache | very active (Sep 2026) |
| `s2n-quic` 1.89 | QUIC | streams + datagrams | TLS 1.3 (`s2n-tls`/`rustls`) | — | — | — | none | Apache-2.0 | very active (Sep 2026) |
| `wtransport` 0.7.2 | WebTransport/H3 on `quinn` | streams + datagrams | TLS 1.3, cert hashes | — | — | — | none | MIT/Apache | active (Aug 2026); "not fully production-ready" |
| `renet`/`renetcode` 2.0 | UDP (pluggable), Steam | 3 channel kinds, frag | netcode 1.02 tokens, ChaCha20-Poly1305 | — | — | — | optional `bevy_renet` | MIT/Apache | active (Jan 2026) |
| `bevy_quinnet` 0.21 | QUIC (`quinn`) | reliable/unreliable channels | TLS; skip / CA / TOFU modes | — | — | — | Bevy | MIT/Apache | active (Jul 2026); no WebTransport |
| `aeronet` 0.21–0.22 | WebTransport, WebSocket, Steam, iroh P2P, channels | per IO layer | per IO layer | — | — | — | Bevy-native | MIT/Apache | active (Jun 2026) |
| `lightyear` 0.30 | UDP, WebTransport, WebSocket, Steam | channels, frag | netcode-style tokens; TLS on WT | not advertised in README | visibility (docs) | yes, rollback + lag comp | Bevy | MIT/Apache | very active (Sep 2026) |
| `bevy_replicon` 0.44 | any (renet/quinnet/aeronet/matchbox) | via backend | via backend | no (change detection) | per-client visibility filters | no (`bevy_rewind` sibling) | Bevy | MIT/Apache | very active (Sep 2026) |
| `naia` 0.25 | UDP, WebRTC | channels incl. tick-buffered | — (DTLS in browser) | yes | rooms / scope | docs for rollback | adapters (Bevy, macroquad) | MIT/Apache | active (May 2026) |
| `matchbox` 0.14 + `ggrs` 0.13 | WebRTC P2P | reliable + unreliable DCs | DTLS | — | — | rollback (lockstep) | `bevy_ggrs` optional | MIT/Apache | active (2026) |
| `laminar` 0.5 | UDP | semi-reliable, frag | — | — | — | — | none | MIT/Apache | dormant since May 2021 |

**Own versus take.** Take: QUIC (`quinn`, `rustls`), the WebTransport server (`wtransport`), the
generic encoder (`bitcode`) for cold messages, the ECS (`bevy_ecs`), the profiler (`tracing-tracy`).
Own: packet layout, quantisation, acked-baseline deltas, the priority accumulator, interest
management, the input jitter buffer and pacing, prediction/rollback, lag compensation, the link
conditioner and the replay format. No crate in the table implements the delta + priority + interest
triad against a replication layer with moving authority, and every gameplay-level crate is coupled to
Bevy's schedule.

---

## 3. Replication, bandwidth and big worlds

**Epic Games. "Replication Graph" and "Iris Replication System." Unreal Engine 5.8 documentation.**
[web] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/replication-graph-in-unreal-engine> ·
<https://dev.epicgames.com/documentation/en-us/unreal-engine/iris-replication-system-in-unreal-engine>

Instead of every actor testing relevance against every connection each tick, actors live in
persistent *nodes* (a spatialisation grid, always-relevant lists, per-connection nodes) that answer
"what should this connection consider" cheaply. Built for Fortnite Battle Royale: "100 connected
players and about 50,000 replicated Actors" at the start of each match. Iris, the successor, is still
marked Experimental in 5.8.
*Bearing:* the shipped, verifiable number for replicated entities per 100 players, and the structure
Forge's RL interest queries should take: a grid of cells with per-client subscriptions plus
always-relevant and owner-only lists, evaluated per node rather than per entity.

**Cloud Imperium Games. "CitizenCon 2953: Shaping the 'Verse — The Future of StarEngine" (segment
"Persistent Entity Streaming, Replication Layer, Server Meshing", Paul Reindell), 23 Oct 2023;
"Server Meshing and Persistent Streaming Q&A" (Reindell, Beausejour, Godfrey, Johnson), RSI
Comm-Link, 2021; "Inside Star Citizen: Alpha 4.0 — Meshing Forward", 14 Nov 2024; "Alpha 4.0 —
Destination Pyro", 19 Dec 2024.** [talk] [web] [recent]
<https://www.youtube.com/watch?v=xKWa4WoTkV4> ·
<https://robertsspaceindustries.com/comm-link/transmission/18397-Server-Meshing-And-Persistent-Streaming-Q-A>
· <https://www.youtube.com/watch?v=Mgbgp4pRSJ4> · <https://www.youtube.com/watch?v=DE3ePRgpUQo>

The public record of the architecture SPEC §10.2 is modelled on: an entity graph persisted in a
database (Persistent Entity Streaming), a **Replication Layer** that owns hot entity state and streams
it by interest, and game servers that hold *authority* over subsets of the graph and hand it over —
the 2023 demo shows a player crossing between two servers while staying connected and a server crash
recovered from the RL. The 2021 Q&A describes static meshing (fixed server-to-region assignment) as
the first step and dynamic split/merge as the second; the December 2024 release of Alpha 4.0 with the
Pyro system is static meshing in production, with dynamic meshing still future work.
*Bearing:* confirms Forge's staging (static partitions with handoff first, dynamic split/merge later)
and is a calendar reality check: CIG's RL plus static meshing took from 2019 to the end of 2024.

**Wikipedia. "Worlds Adrift" and "Improbable (company)."** [web] [still-current]
<https://en.wikipedia.org/wiki/Worlds_Adrift> · <https://en.wikipedia.org/wiki/Improbable_(company)>

The SpatialOS record, as the warning: SpatialOS (open beta Feb 2017, $502M Series B May 2017)
promised generic worker-based distributed simulation; Worlds Adrift (Bossa, early access July 2017)
shut on 26 July 2019 as "no longer commercially viable"; Mavericks was cancelled in 2019, Scavengers
shut in 2022 before leaving early access; a 2019 terms dispute with Unity blocked SpatialOS games
until Epic and Improbable set up a $25M fund; by 2023 Improbable had pivoted to MSquared.
*Bearing:* a generic "any worker, any entity" mesh without game-specific partitioning and contact
rules is expensive and hard to make feel good; a platform dependency is a business risk; the game must
be fun before the mesh is.

**CCP Veritas. "Time Dilation Video Demo." EVE Online dev blog, 30 Sep 2011.** [web] [foundational]
<https://www.eveonline.com/news/view/time-dilation-video-demo>

EVE's single-shard answer to overload: when a solar-system node cannot keep up, *simulation time*
slows for everyone in it so every command still resolves in order, instead of timing out. Announced
for public testing here; in production since.
*Bearing:* Forge's per-partition TiDi (SPEC §10.3 item 6) is this; implement it as a scale on the
tick's dt and client-visible clocks, never as dropped ticks.

**Richard M. Fujimoto. *Parallel and Distributed Simulation Systems*. Wiley, 2000. ISBN
978-0-471-18383-9.** [book] [foundational]
<https://www.wiley.com/en-us/Parallel+and+Distributed+Simulation+Systems-p-9780471183839>

The textbook for distributed discrete-event simulation: conservative versus optimistic (Time Warp)
synchronisation, lookahead, global virtual time, DIS/HLA. Server meshing is a special case: workers
are logical processes, ghost overlap is lookahead, authority epochs are a conservative protocol.
*Bearing:* the vocabulary for reasoning about handoff correctness, and the source to cite when
someone proposes optimistic cross-worker interaction.

**Clockwork Labs. `SpacetimeDB`. BSL 1.1 → AGPL, 2023–2026.** [code] [recent]
<https://github.com/clockworklabs/SpacetimeDB>

"A relational database that is also a server": game logic runs as modules (Rust, C#, TypeScript,
C++) inside the database, clients subscribe to queries and receive pushed updates; claims BitCraft
Online's whole backend is one module.
*Bearing:* the strongest alternative to a hand-built RL for transactional, low-rate state (inventory,
market, social); not a substitute for a 60 Hz physics worker, which is why SPEC §10.2 keeps it as a
candidate for services only.

---

## 4. Prediction, reconciliation and lag compensation

**Matt deWet, David Straily. "Peeking into VALORANT's Netcode" (28 Jul 2020); Brent Randall,
"VALORANT's 128-Tick Servers" (31 Aug 2020). Riot Games Technology.** [web] [recent]
<https://www.riotgames.com/en/news/peeking-valorants-netcode> ·
<https://www.riotgames.com/en/news/valorants-128-tick-servers>

A 128-tick server with fixed-timestep movement shared by client and server, about one frame of
buffering on the client and half a frame on the server, and rewind to the moment of the shot; peeker's
advantage cut by ~40 ms (28 %), and a 10 ms difference moved a controlled duel from 90 % defender win
rate to the attacker. The cost: a per-frame server budget under 2.34 ms with three matches per core on
36-core hosts, reached by converting replicated properties to RPCs, cutting animation work by 75 %,
and tuning CPUs, NUMA and the Linux scheduler.
*Bearing:* buffer depth is a gameplay variable, so Forge's jitter buffer must be measured and tuned
per client; and the per-tick budget is set by hardware and player count, so Forge's 60 Hz workers on
an 8-thread machine need the same Tracy-driven budget work (§6).

**Shengmei Liu, Xiaokun Xu, Mark Claypool. "A Survey and Taxonomy of Latency Compensation Techniques
for Network Computer Games." ACM Computing Surveys, 2022 (companion page at WPI).** [paper] [recent]
<https://web.cs.wpi.edu/~claypool/papers/lag-taxonomy/LatencyCompensation.html>

The peer-reviewed map: speculative execution (prediction, extrapolation), time warp (rewind), latency
concealment and adjustment/assistance, each placed at client, server or either; the index of the
academic literature behind each technique.
*Bearing:* the citation for a technique's provenance when a GDC talk is not enough.

Cross-references: Bernier (§1) for mechanics; Ford/Overwatch (§1) for rollback of predicted state and
server-driven pacing; Cone/Rocket League (§1) for predicting physics objects; Fiedler "State
Synchronization" (§1) for smoothing corrections by quantising on both ends.

---

## 5. Determinism and the simulation

**Glenn Fiedler. "Floating Point Determinism" (24 Feb 2010) and "Deterministic Lockstep" (29 Nov
2014). Gaffer On Games.** [web] [foundational]
<https://gafferongames.com/post/floating_point_determinism/> ·
<https://gafferongames.com/post/deterministic_lockstep/>

Why identical source does not give identical floats across compilers and CPUs (x87 precision, FMA
contraction, library `sin`/`exp`, fast-math): achievable on one compiler/architecture with discipline,
expensive and fragile across platforms. The lockstep article shows inputs-only networking with a
playout-delay buffer and *redundant inputs in every UDP packet* beating TCP at 25 % loss and 2 s
latency — and its own demo desyncing between machines because floats differ.
*Bearing:* Forge needs *same-binary* determinism (replays, tests, two-worker mode), not cross-machine
determinism; and the redundant-input packet is what Forge's input channel should do regardless.

**Erin Catto. "Determinism." box2d.org, 27 Aug 2024.** [web] [recent]
<https://box2d.org/posts/2024/08/determinism/>

Box2D v3 is deterministic by default at three levels, including multithreaded (bit arrays fix the
order instead of atomics, since "an atomic or mutex … indicates a risk to determinism") and
cross-platform: no fast-math, FMA contraction disabled by compiler flag, its own trig functions —
judged far better than fixed point, which would be slower and shrink the usable world.
*Bearing:* the recipe for a deterministic Rust sim: no atomics in simulation order, explicit
`mul_add` or none, own transcendentals; and evidence that fixed point is not required.

**Jorrit Rouwe, `JoltPhysics` (MIT); Dimforge, "Determinism" (Rapier user guide).** [code] [web]
[still-current]
<https://github.com/jrouwe/JoltPhysics> · <https://rapier.rs/docs/user_guides/rust/determinism>

Jolt (Horizon Forbidden West, Death Stranding 2) states the simulation "runs deterministically" so a
remote copy can be driven by inputs alone, with a pointer to its Deterministic Simulation section for
the limits (same build; cross-platform only under conditions). Rapier is "locally deterministic" by
default and needs the `enhanced-determinism` feature, IEEE 754-2008 platforms and `nalgebra`'s
`RealField` math (not `std` methods) for cross-platform runs. See the physics research file.
*Bearing:* same-build determinism comes free with either; cross-platform is not promised by either
and should not be relied on.

**Rust project. `f32::sin`/`f32::exp` documentation; `rust-lang/libm`.** [spec] [code]
[still-current]
<https://doc.rust-lang.org/std/primitive.f32.html> · <https://github.com/rust-lang/libm>

The standard docs say of `sin`, `exp` and friends: "The precision of this function is
non-deterministic. This means it varies by platform, Rust version, and can even differ within the
same execution from one invocation to the next." `mul_add` is guaranteed correctly rounded, and Rust
never contracts `a*b+c` into an FMA on its own. `libm` (MUSL port, pure Rust) gives
platform-independent results; the standalone repo was archived in April 2025 and moved into
`compiler-builtins`, but the crate remains published.
*Bearing:* every function on Forge's replay-critical path uses `libm` (or Forge's own)
transcendentals, never `std`'s; use `mul_add` explicitly or not at all, consistently on client and
server.

**Exit Games. "Photon Quantum 3 — Intro." doc.photonengine.com, 2026. — Wube Software, "Friday
Facts #147 — Multiplayer rewrite" (15 Jul 2016) and "#302 — The multiplayer megapacket" (5 Jul
2019).** [web] [recent] [still-current]
<https://doc.photonengine.com/quantum/current/quantum-intro> ·
<https://www.factorio.com/blog/post/fff-147> · <https://www.factorio.com/blog/post/fff-302>

The shipped deterministic predict/rollback engine: sparse-set ECS, deterministic math/physics/
navigation libraries, clients exchanging only inputs, a game-agnostic server that synchronises clocks
and manages input latency so nobody waits for the slowest client; up to 128 players. Factorio is the
best-documented lockstep game: a server merging every player's inputs into one tick package
(O(n²) → O(n) packets), local prediction of one's own actions, and a post-mortem of a corrupted
latency queue producing packets of 400+ actions that took servers down.
*Bearing:* Quantum's server-managed input latency is the same pacing idea as Overwatch's; the
megapacket story is the input-queue lesson again — bound the queue, never replay it, make the server
drop rather than amplify when a client misbehaves.

---

## 6. Server architecture for a living world

**Bevy Engine. `bevy_ecs`. MIT/Apache, 0.19 stable / 0.20 rc (Sep 2026).** [code] [still-current]
<https://lib.rs/crates/bevy_ecs>

Archetype ECS with parallel scheduling, change detection, observers and relations; documented as
usable "as a standalone crate" without the engine (~680k downloads/month).
*Bearing:* the worker's ECS; change-detection ticks are the natural "dirty since tick T" source for
the RL, and the schedule runs from Forge's own fixed-tick loop, not Bevy's app runner.

**Bartosz Taudul. `Tracy`; `tracing-tracy` 0.12 (Aug 2026).** [code] [still-current]
<https://github.com/wolfpld/tracy> · <https://lib.rs/crates/tracing-tracy>

Nanosecond-resolution frame/sampling profiler with remote telemetry over the network, plots, memory
and lock tracking; `tracing-tracy` turns Rust `tracing` spans into Tracy zones.
*Bearing:* the headless server profiles into a Tracy client on the dev PC; per-tick budget graphs are
the acceptance test for every server stage below.

**Synadia, "What is NATS?"; Broadcom, "RabbitMQ Streams"; Redis, "Redis Streams."** [web]
[still-current]
<https://docs.nats.io/nats-concepts/overview> · <https://www.rabbitmq.com/docs/streams> ·
<https://redis.io/docs/latest/develop/data-types/streams/>

Three bus families: NATS (single small binary, sub-millisecond pub/sub, JetStream for persistence),
RabbitMQ (AMQP queues with destructive consumption, plus *Streams* — a persistent replicated
append-only log with non-destructive consumers over a dedicated binary protocol), Redis Streams
(append-only log with consumer groups, `XACK`, `XCLAIM`). All add a broker hop, per-message framing
and persistence latency that have no place in a 60 Hz loop.
*Bearing:* RabbitMQ stays for durable asynchronous events as SPEC §10.5 says; if Forge later needs
low-latency fan-out for telemetry or presence, NATS core is the lighter tool; nothing per-tick crosses
a broker.

**MongoDB. "Transactions." MongoDB Manual, 2026.** [web] [still-current]
<https://www.mongodb.com/docs/manual/core/transactions/>

Multi-document ACID transactions require a replica set or sharded cluster (never a standalone),
default to a 60-second lifetime, and cost more than single-document writes; the manual's advice is to
model so most updates are single-document.
*Bearing:* the Pi's MongoDB runs as a single-node replica set from day one (SPEC Q-19); write-behind
batches from the RL are plain upserts; transactions are reserved for trades and purchases.

**jagt, `clumsy` (Windows, MIT); Linux `tc-netem(8)`.** [code] [web] [still-current]
<https://github.com/jagt/clumsy> · <https://man7.org/linux/man-pages/man8/tc-netem.8.html>

OS-level link conditioners: clumsy intercepts packets through WinDivert and adds lag, drop, throttle,
duplicate, reorder and tamper interactively; netem is the Linux queue discipline for delay, jitter,
loss, corruption, duplication, reordering and rate.
*Bearing:* use them for whole-machine tests (the LAN server behind a conditioned link); Forge also
needs an in-process conditioner inside its transport trait so CI can run the 100-bot soak without
admin rights, as `yojimbo` and GameNetworkingSockets do.

---

## 7. Security and crypto

**Martin Thomson, Sean Turner. "Using TLS to Secure QUIC." RFC 9001, IETF, May 2021.** [spec]
[still-current]
<https://www.rfc-editor.org/rfc/rfc9001.html>

QUIC's handshake *is* TLS 1.3: one round trip to full keys, 0-RTT resumption with the documented
replay risk (applications decide which frames are safe in early data). Certificates, key schedule and
packet protection come from a mature stack (`rustls`) rather than a game library.
*Bearing:* TOFU-pinned self-signed certificates are a legitimate TLS mode (SSH-style pinning); public
servers move to CA certificates plus a signed login token on the first stream; never put game inputs
in 0-RTT data.

**Glenn Fiedler, "netcode 1.02 Standard" (`STANDARD.md`); Trevor Perrin, "The Noise Protocol
Framework", rev. 34, 11 Jul 2018.** [spec] [still-current]
<https://github.com/mas-bandwidth/netcode/blob/main/STANDARD.md> · <https://noiseprotocol.org/noise.html>

The pre-shared-key alternative to TLS: the private connect token is sealed with XChaCha20-Poly1305
under a key shared between web backend and servers; challenge/response proves possession; every
later packet is ChaCha20-Poly1305 with a 64-bit sequence number as nonce (server sequence starts at
2⁶³ so nonces stay disjoint); seven packet types, no certificates. Contrary to folklore, netcode.io
does **not** use Noise; Noise is the separate framework of Diffie-Hellman handshake patterns (`XX`,
`IK`, …) behind WireGuard, for certificate-free channels with static keys.
*Bearing:* borrow the token *contents* (client id, expiry, allowed servers, user data) and the rule
that the game server never calls the auth service on connect; Noise is the right primitive only if
Forge ever builds a certificate-free raw-UDP link (worker↔RL on a LAN).

Cross-references: RFC 9000's 3× anti-amplification limit and address validation (§2) are the
transport's DDoS floor; Steam Datagram Relay (§2) hides server IPs; input validation (speed caps,
server-side ability checks, no client authority over anything but intents) is SPEC §10.4 and
Bernier (§1).

---

## 8. Voice

**Jean-Marc Valin, Koen Vos, Timothy Terriberry. "Definition of the Opus Audio Codec." RFC 6716,
IETF, September 2012.** [spec] [foundational]
<https://www.rfc-editor.org/rfc/rfc6716.html>

The codec every game voice system uses: 6–510 kbit/s, 2.5–60 ms frames, designed for interactive use
including "in-game chat"; low algorithmic delay, loss-tolerant.
*Bearing:* in-house voice would be Opus at 20 ms frames over an unreliable QUIC datagram channel with
positional mixing on the client; otherwise EOS Voice (§2) or Steam's voice API.

---

## Recommendation for Forge

**Transport.** Stay on QUIC via `quinn` with `rustls`: unreliable datagrams for inputs and snapshots,
one reliable stream per event channel (world events, chat, RPC-style requests), a control stream for
handshake and clock sync. Wrap it in a `Transport` trait with three implementations from the start —
`quinn`, an in-process loopback with a link conditioner (loss, jitter, duplication, reordering,
bandwidth cap), and later `wtransport` for browsers — so the same protocol bytes run on all three.
Keep a raw-UDP `renetcode` backend as a documented option if QUIC's per-packet overhead ever shows in
profiles; do not build it now. Certificates: TOFU pinning for LAN and dev, CA certificates for public
servers; the first message on the control stream is a netcode-style token (client id, expiry, server
allow-list, user data) signed by the web login service, verified offline by the server.

**Replication model.** Simulation at 60 Hz on workers; the RL sends each client snapshots at 30 Hz near
the player and lets the priority accumulator starve distant entities to 5–10 Hz naturally. Per client:
a byte budget (start at 24 KB/s ≈ 800 bytes per 30 Hz packet, one datagram under the 1200-byte floor),
a priority accumulator over the client's interest set (priority = importance × f(distance, size) +
staleness, boosted for owned and targeted entities), and a ring of the last 32 acknowledged baselines
per entity. Packet layout, in order: header {protocol tag/version, server tick, last input tick
received from this client, baseline tick acked, client time echo}; priority-ordered entity blocks
{entity id as delta-varint from the previous block, component change mask, quantised fields}; a
trailing "removed / out of interest" list. Quantisation: positions relative to the entity's cell
origin (cube-sphere tile or space octree cell) at 1 cm for walkers and 1 dm for capital ships,
smallest-three quaternions at 3 × 9 bits, velocities on a bounded log scale, booleans and small enums
as raw bits — a hand-written bit writer, not `bitcode`. Full state when the acked baseline is older
than the ring. `bitcode` for the reliable streams and for RL↔worker traffic on the LAN.

**Inputs, prediction, pacing.** Clients send inputs at 60 Hz with the last four inputs redundantly per
datagram. The worker keeps a per-client jitter buffer with a *measured* target depth (start at two
ticks) and reports its fill level in each snapshot header; the client nudges its clock to hold the
target (Overwatch pacing). A starved buffer waits, never repeats. The client predicts its own
avatar/ship and any physics group it is touching, keeps predicted components in a rollback store keyed
by tick, reconciles only when the error exceeds a threshold (1 cm / 0.5°), and fades corrections over
~100 ms. Interpolation for others: 100 ms behind the newest snapshot, 150 ms for the 20 Hz tier. Lag
compensation: a one-second history ring of hit volumes per entity on the worker; shots trace at
`client_time − interp`. The server validates every intent — speed caps per mobility mode, cooldowns,
reach — and never accepts a client-authored position.

**Determinism.** Fixed 60 Hz, commands applied in a deterministic order, `libm` transcendentals and an
explicit `mul_add` policy on client and server, no atomics on the simulation order. Not required for
state sync; required for replays, the sync-test harness and the two-worker mode.

**Server pieces and where the infrastructure fits.** Worker = tokio for I/O plus one dedicated sim
thread running `bevy_ecs` from Forge's own fixed-tick loop; several workers per process on the
8-thread server now, one per process once the orchestrator exists (same protocol either way). RL =
separate process, sharded by partition, holding entity state and per-client interest/priority state;
clients connect only to the RL. Persistence = a consumer of the RL's change stream doing write-behind
upserts to MongoDB (single-node replica set on the Pi) every few seconds, transactions only for trades.
RabbitMQ carries persistence jobs, economy, chat, orchestrator commands and telemetry summaries;
nothing per tick. Observability: `tracing` spans into Tracy from the headless server, counters per
client (bytes, loss, corrections, buffer depth). Replays: record every client input and every
cross-worker command with its tick plus a full snapshot every 10 s; a replay must reproduce the RL
stream bit-for-bit under the determinism rules. Bots: the client crate with a scripted controller and
no renderer.

**Order of work and the proof for each stage.**

1. *Baseline (M0/M1, done):* QUIC, `bitcode` snapshots at 30 Hz, prediction of walkers, two-worker
   handoff. Proof: two players at 100 ms RTT and 2 % loss hand over cleanly.
2. *Bandwidth:* quantisation, acked-baseline deltas, priority accumulator, cell-based interest.
   Proof: 100 headless bots on the LAN server through the in-process conditioner (100 ms RTT, 20 ms
   jitter, 2 % loss) for 10 minutes at ≤ 24 KB/s per client, with bytes/entity/tick logged; the
   Fiedler-scale expectation is under one byte on average.
3. *Feel:* adaptive input pacing, correction thresholds and smoothing, lag compensation. Proof: eight
   players with RTTs from 20 to 250 ms; a hit-registration harness firing at a moving target at known
   times and asserting server hits; zero corrections of 0.000 m in the logs.
4. *Scale of authority:* 200 bots patrolling across a two-worker boundary continuously, epoch
   assertions in the RL (no write accepted with a stale epoch), one worker killed and restarted
   mid-run, and the run's replay diffed against the live RL stream.
5. *Reach:* `wtransport` browser client on the same protocol; web-login tokens; then Steam Datagram
   Relay or an equivalent relay once there is something public to attack.

---

## Checked and left out

Kept so the bibliography is auditable: looked for and not cited above, with the reason.

- **A PlanetSide 2 engineering talk on its 2,000-player continents.** Nothing citable was found;
  general search returned only connection-troubleshooting material. If one exists it is likely in the
  GDC 2013–2014 programming track; verify before citing.
- **CCP Veritas, "Introducing Time Dilation" (2011).** Widely referenced, but the eveonline.com URL
  now returns "page not found"; the companion "Time Dilation Video Demo" post is reachable and cited.
- **A CitizenCon 2954 (2024) server-meshing talk.** The 2954 engineering keynote on the official
  channel ("Brave New Worlds") is about Genesis planet tech; 2024 meshing coverage was in the "Inside
  Star Citizen" episodes cited above.
- **Ashes of Creation / Improbable "M²" server technology.** No engineering-grade public source;
  press coverage only.
- **A formal Worlds Adrift post-mortem (GDC or Game Developer).** Not located; Wikipedia carries the
  dates and stated shutdown reason and is cited as the record.
- **Halo: Reach bandwidth/priority figures and Rocket League tick rates.** The GDC Vault entries
  confirm the talks; specific numbers often quoted from the slides were not re-verified and are not
  repeated.
- **`lightyear` delta compression and rooms.** Present in the project's docs in earlier versions but
  not in the README checked; recorded in the table as "not advertised" / "docs" rather than asserted.
- **A "QUIC for games" measurement paper (HOL blocking, datagram overhead versus raw UDP).** None of
  RFC-level authority was found; the overhead question is answered empirically by stage 2.
- **Unreal Iris production status.** Epic's 5.8 docs still mark Iris Experimental; claims that
  Fortnite runs it in production were not confirmed from Epic's pages and are not made.
- **Valve's "Latency Compensating Methods" wiki mirror.** developer.valvesoftware.com blocks
  automated fetches (HTTP 403); Bernier is cited through Claypool's taxonomy page, which carries the
  citation line and links the gamedevs.org PDF.
- **`s2n-quic`, `bevy_quinnet`, `aeronet`, `wtransport`, `laminar` as separate entries.** Verified
  (GitHub READMEs, lib.rs versions) but folded into the table and the `quinn` entry to keep the list
  near 45 (it stands at 48).

---

## Verification notes

Every entry was checked on 23–24 September 2026 against at least one reachable page: the GDC Vault
listing for each talk (title, speaker, company, year — note that GDC Vault dates "8 Frames in 16ms" to
GDC 2018, not 2019 as it is often cited), the author's site for each Gaffer On Games post (title and
date), the RFC Editor for RFCs 6716, 8899, 9000, 9001 and 9221, the W3C TR page and the IETF
datatracker for WebTransport (revision and status as of July 2026), GitHub READMEs for every library
and lib.rs for crate versions and dates (crates.io pages render client-side and returned nothing).
The Tribes and Bernier PDFs came back as binaries that could not be rendered here; both were confirmed
through pages that describe and link them (archive.org's item page with authors and date; Claypool's
taxonomy page with the citation line). Web search quota ran out midway; the remaining lookups used
direct URLs. Three sources could not be read by fetch and were confirmed in a browser session opened
*before* the owner's instruction not to use the browser pane arrived, and none after: the Valve
Developer wiki (HTTP 403 to fetches; the tick, update-rate and `cl_interp` figures were read from the
rendered page), the RSI Comm-Link (cookie wall; non-essential cookies were declined and the article
header naming the panellists was read), and the three official Cloud Imperium YouTube videos (titles,
channel and publish dates 23 Oct 2023, 14 Nov 2024 and 19 Dec 2024 read from page metadata; a YouTube
tab was left open in the pane). The Photon Quantum documentation sits behind a bot check for fetches
and was read the same way; its marketing page was also fetched directly. Anything that could not be
reached is under "Checked and left out" rather than cited from memory. In-house findings (WebSocket
head-of-line stalls, the starved-queue rule, the 0.000 m correction bug) are the project's own and
carry no citation.
