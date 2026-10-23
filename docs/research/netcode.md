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
9. [Patents met](#9-patents-met)
10. [Recommendation for Forge](#recommendation-for-forge)
11. [Checked and left out](#checked-and-left-out)
12. [Verification notes](#verification-notes)

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
*Built (#268, 2026-10-16):* `forge-net` on `quinn` 0.11.12 (`quinn-proto` 0.11.19, `quinn-udp`
0.5.16), `rustls` 0.23.46 and `tokio` 1.53.3, resolved that day. The crypto provider is `ring`
0.17.14 rather than rustls's default `aws-lc-rs`: it is quinn's own default (`rustls-ring`), it
gives Ed25519 for the connection tokens, and it builds without CMake or NASM (its Windows assembly
ships as objects and `cc` finds MSVC through the registry; the dependencies built cold in 18 s on the
dev machine). The tests do not drive `quinn-proto` from a simulator after all: quinn's
`AsyncUdpSocket` trait takes a socket of ours, so the seeded conditioner sits under the real
endpoint and the tests run the whole stack on loopback (`docs/demos/net.md`).

**Glenn Fiedler. "Packet Fragmentation and Reassembly." Gaffer On Games, 6 Sep 2016.** [web]
[foundational]
<https://gafferongames.com/post/packet_fragmentation_and_reassembly/>

A packet larger than the MTU sent as fragments, each with the packet's 16-bit sequence, its index
and the count; fire and forget, so one lost fragment loses the packet; incomplete entries cleaned up
from a sequence buffer.
*Bearing:* RFC 9221's datagrams are never fragmented, so an unreliable message over ~1200 bytes needs
exactly this; `forge_net::fragment` follows it with a 5-byte header and a one-second timeout.

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

## 9. Patents met

Searched on 16 October 2026 for #268 (the owner's rule: number, holder, jurisdiction, status,
expiry, an alternative). Each was read on its Google Patents page; the statuses and expiries are
Google's estimates, not legal conclusions, and claim readings here are an engineer's, not a lawyer's.
The web search tool found few patents by technique; the pages below were reached by number or by a
search hit. Lag compensation by server-side rewind turned up no patent at all in these searches,
which is not evidence that none exists: search again before step 3 builds it.

| Patent | Holder | What it claims, roughly | Status | Bearing |
|---|---|---|---|---|
| **US 11,810,245 B2**, "Methods and systems for prioritized replication of a virtual world" (filed 2021-08-25, granted 2023-11-07) | Verizon Patent and Licensing | on a client's request to join a virtual world, priorities for assets from the avatar's location; the first asset's initial data, then its update data, and only after that the second asset's initial data (claims 1, 14, 20 all start from the join request) | active, US, anticipated expiry 2041-12-31 | **Live, on step 2's path.** A priority accumulator that, after a join, sends updates of near entities before the first state of far ones reads on the claimed order. Prior art is older (Tribes 1999, Quake III's snapshots, Unreal's actor priority, Fiedler 2015), but a granted patent is presumed valid. The alternative: every entity entering a client's interest gets its first full state before any update is sent to that client (a join's whole state first, as Quake III sends its gamestate on connect), so no update ever precedes another entity's initial data. *Built that way* (#268, `forge_net::replicate`, "initial states first", with a test that no packet carries a delta while a first state waits): the owner is asked to confirm the reading |
| **US 10,579,434 B2**, "Simulation systems and methods using query-based interest" (priority 2016-08-24, granted 2020-03-03), and its family: US 10,025,892, US 10,643,010, US 11,533,367, US 11,936,734, application US 2024/0314206 | Improbable Worlds | an entity carries an interest component: a query subscription to the entity database and the frequency of its updates, changeable at run time | active, US, '434 to 2036-11-28 | Already flagged in #250. The alternative, D-010's and #250's: interest held by the replication layer per client as grid cells, never a query component on the entity |
| **US 12,194,388 B2**, "Method and apparatus for providing a computer implemented game" (priority 2022-06-30, granted 2025-01-14) | King.com | the server receives a client's input and its own simulation of an entity, simulates it itself, and when the two differ applies a bias toward the client's result (a blend within a threshold) | active, US, anticipated expiry 2042-09-16 | Not on Forge's path: the server never takes a client-authored state (D-010, §4); reconciliation happens on the client. Keep it so |
| **US 7,627,632 B2**, "Reducing bandwidth requirements for peer-to-peer gaming based on importance of remote objects to a local player" (priority 2006-11-13) | Microsoft | the most important remote players get continuous state updates, the others periodic ones simulated in between | expired (fees not paid; it would have run to 2028-05-20) | Free to use: importance-scaled update rates |
| **US 6,628,287 B1**, "Method and apparatus for consistent, responsive, and secure distributed simulation in a computer network environment" (priority 2000-01-12) | There, Inc. (now Leidos) | an authoritative server; each client shows its own avatar ahead of global virtual time from the same timestamped inputs, the others behind it, and the server repairs divergence | expired 2020-01-12 | Free to use: client prediction ahead of an authoritative server, others interpolated behind |
| **US 2014/0365412 A1**, "Method and system for improving prediction in online gaming" (priority 2012-02-01) | Yosef Mizrachi | predicting a player's command from sensor patterns before it is entered | abandoned 2017 | None |
| **US 10,940,388 B1**, "Variable latency compensation for asynchronous multiplayer games" (priority 2017-10-27, granted 2021-03-09) | Amazon Technologies | claim 1: the variance between a client's input packets' arrival and their expected rate, the velocity of the character it controls, a hit volume from that velocity and variance, the character shown at a smoothed place, the game's interactions taken against the hit volume about that place; claim 5: a gameplay aspect adjusted from the timing variance, the velocity and a velocity-based hit volume | active, US, anticipated expiry 2038-03-10 | **Live, near step 3.** Read 2026-10-23. A plain server-side rewind (a history ring, the world moved back to the tick the shooter saw, the shot tested) measures no input-timing variance and sizes no hit volume by it: outside both independent claims. Keep it so: never grow or move hit volumes, nor the drawn places, by measured packet jitter |
| **US 11,771,984 B2**, the same family's continuation (filed 2021-03-08) | Amazon Technologies | claims 1, 8, 15: the variance between a client's input rate and the rate expected from its earlier inputs, a character's place and velocity from its inputs, the character shown at a place set from the variance, the place and the velocity, the interactions taken against that shown place | active, US, anticipated expiry 2037-11-04 | As the parent: every independent claim needs the measured input-rate variance. Forge's interpolation delay is fixed (100 ms), never set per client from its jitter |
| **US 8,678,929 B1**, "Client-side prediction of a local game object to reduce apparent network lag of multiplayer simulations" (priority 2008-08-01) | Electronic Arts | a local object predicted from local input, shared objects course-corrected toward a peer-to-peer network simulation's state of an earlier frame | expired, fees not paid | Free to use |
| **US 6,475,090 B2**, "Compensating for network latency in a multi-player game" (priority 2001-03-29) | Philips | latencies measured and a player's playing modality adjusted to advantage one over another (armour, visibility) | expired 2021 | Free to use; not a technique Forge plans |

**Lag compensation, searched again for step 3** (2026-10-23, the search §9's first paragraph asked
for): web searches by technique (server-side rewind, hit registration, historical hit boxes, the
large shooters' holders) found no patent on rewinding other players' hit volumes to the time the
shooter saw them. Bernier's GDC 2001 paper and Source's shipped netcode are the prior art, a
quarter of a century old. The only live claims near it are Amazon's two above, which need the
input-timing variance. As before, not finding one is not evidence that none exists; the owner
decides before a sold product.

**Authority handoff between workers, searched before building it** (2026-10-23, for step 2's
remainder). A research agent read the independent claims on Google Patents. The fetch tool cut
quotes short, so the readings are close paraphrases; claim 1 of US 10,878,146 was read again
here. Statuses and expiries are Google's estimates.

| Patent | Holder | What the independent claims need | Status | Bearing |
|---|---|---|---|---|
| **US 10,878,146 B2**, "Handover techniques for simulation systems and methods" (priority 2016-08-24) | Improbable Worlds | authority over components given to a worker; a handover decided; **the first worker told that authority will move after a period of time**; the second given it; both told | active, US, about 2037-07-15 | Read only by an advance notice. Forge's layer moves authority at a tick's end with no notice and no handshake: the old owner finds the entity gone from its picture at the next barrier, and its late writes are refused by the epoch. Keep it so |
| **US 10,289,767 B2**, "Communications interface facilitating operations of a persistent spatially-optimized computer-based simulation" (2016-08-24) | Improbable | workers by spatial region, sent add-entity, remove-entity, component-state-change and delegate-authority-over-a-component messages | active, about 2037-07-15 | Kept clear: authority is whole-entity (one epoch an entity, no component named), and a worker takes one picture of the layer each tick (owned with epochs, ghosts), working out what came and went itself. No such message API |
| **US 10,839,116 B2** and **US 11,816,402 B2**, "Simulation systems and methods" | Improbable | the combination: chunk modules that monitor a region's entities and migrate them, a bridge a worker, a receptionist, an oracle; recovery from snapshots; balancing | active, to about 2036–2038 | Kept clear: one layer with a flat table, regions as data, workers talking to the layer directly |
| **US 10,380,282 B2**, **US 11,550,621 B2**, pending **US 2023/0090130 A1** (load-balancing via partitions and virtual processes) | Improbable | partitions sent to partition-enforcer services and virtual processes to a manager; one entity's components split between processes | active (the application pending) | Kept clear: the layer owns the static table, workers are processes started with their region, authority whole-entity |
| **US 11,416,305 B2**, "Commands for simulation systems and methods" | Improbable | a command to a component another worker owns, routed through bridges, the response carrying the command's id | active, about 2037-10-13 | Kept clear: interactions across a border are read from the ghosts; events, when they come, one-way and addressed to an entity, no response |
| **US 12,386,655 B2**, "Event-based view synchronization" | Improbable | a runtime ticking faster than its workers, finding authority changes in a tick | active, about 2044 | Kept clear: the layer and the workers share one tick |
| **US 10,474,655 B1**, **US 11,301,447 B2**, "Entity database" | Improbable | a client subscribes to a query; queries kept with ids in an index by region | active, about 2038 | The client side of the layer: interest stays the server's, cells per client, no client queries |
| **US 12,681,773 B2**, **US 12,636,578 B2**, **US 11,909,601 B1**; **US 10,303,821 B2** | Amazon; Improbable | partitions moved between workers or applications by load, workers scaled; a volume split off to a new node by load | active, to about 2042–2044 | Not this stage (static regions). Moving regions by load later needs its own search and design-around |
| pending **US 2025/0239013 A1**, and **US 12,654,102 B2** | Tencent | proxies made in both spaces for a transfer and recycled after; a frame around the object, a map-cut request | pending; active | Ghosts here are standing read-only views of a band, not made per transfer. Watch the application |
| **US 8,612,196 B2** (2002) | Linden Research | regions on servers; the claims: changes sent when perceptible at the display's resolution | active to about 2028-06 | Its 2003 publication is prime prior art for ghosts promoted on crossing. Never filter updates by display resolution before 2028 |

Never moving a client's connection to a worker, nor promoting a client to simulate, stays clear
of Microsoft's US 10,348,836, Amazon's US 10,960,304 and Build A Rocket Boy's US 12,544,661.
**Prior art** for the parts built: HLA's ownership transfer (1998, IEEE 1516-2000) and DIS's
Transfer Ownership PDU (IEEE 1278.1a-1998); Linden's regions with border ghosts (US 2003/0195735);
BigWorld's real and ghost entities; Colyseus (NSDI 2006); EVE's node mapping; Fujimoto (2000) for
conservative synchronisation with lookahead; fencing by epochs and terms (Viewstamped Replication
1988, leases 1989, Paxos 1998, Chubby's sequencers 2006, ZooKeeper 2010, Raft 2014). No patent
claiming stale-epoch write rejection in games was found. No search reached CN, KR or JP filings in
depth (Tencent and NetEase file heavily on seamless worlds there), and not finding a patent is not
evidence that none exists.

**Input pacing, prediction and smoothing, searched before building the pacing** (2026-10-23).
A research agent read the claims on Google Patents (close paraphrases); claim 1 of US 9,479,618
was read again here. No claim was found on the pacing itself: the server reporting how early
inputs arrive, the client running a little faster or slower to hold a target.

| Patent | Holder | What the independent claims need | Status | Bearing |
|---|---|---|---|---|
| **US 9,479,618 B2** (priority 2014-03-25) | Google (from CiiNow) | on each new input command, the oldest in a ring buffer removed, the others shifted, the new one appended, and the ring's commands sent in one UDP message | active, US only, about 2035-01-16 | **Kept clear:** Forge's client sends the oldest commands not yet acknowledged and drops one only when the server acknowledges it, never because a new one came. Quake 2 (1997) and Quake 3's `cl_packetdup` (1999) are prior art. Never make the redundancy a fixed window of the last N |
| **US 2026/0108808 A1** "Flexible netcode architecture" (priority 2024-10-23) | Roblox | an authoritative update; a rollback check against the prediction; roll back and apply; predict; reconcile the last displayed position with the updated one; render | **pending** | Reads as filed on prediction with smoothed corrections, which Quake (1996–97), Overwatch and Fiedler made public long before. Not infringeable unless granted: watch it, and keep these notes dated. The lab fades its corrections since 2026-10-23 (Quake 2 faded its prediction error over a server frame in 1997): look again if it is granted |
| **US 10,942,588 B2** "Latency adjustment of user input devices" (2019-03-02; EP3934774B1) | Microsoft | a host sets a pace for an input device's reports and tells the device its interval and offset | active, about 2039 | Kept clear: the server sends a measure only, never a pace, interval or offset, and no device is retimed |
| **US 12,290,753 B2** "Adaptive time dilation based on player performance" (2019-11-05) | Sony | presentation slowed for a player's slow reactions, sped back up if they stay slow | active, about 2042 | Kept clear: the rate follows the lead's error alone |
| **US 11,700,402 B1**, **US 12,081,812 B2** (2022) | NVIDIA | a render rate set from a receiver's frame-buffer occupancy | active, about 2042 | Not the technique (frames for display); the simulation's clock alone is paced, never the display's vsync |
| **US 11,033,813 B2** "Latency erasure" (2019) | Microsoft | the client returns its input with the server state it saw; the server applies it to that earlier state | active, about 2039 | Not on the path: inputs are applied at their stamped tick. Lag compensation by a state the client echoes would need a look first |
| **US 11,876,685 B1**, **US 12,401,574 B2** (2021) | Amazon | predicted-state data (a hash) sent with the inputs; the server compares it with its own to decide a correction | active, about 2041 | Kept clear: the client compares the server's digests; nothing predicted goes upstream |
| **US 12,157,057 B2** (2020) | Tencent | a correction chosen by comparing the deviation with a maximum from the server's update window and the motion | active, about 2041 | Kept clear (2026-10-23): the lab fades a correction by a fixed time (5 % left after 0.1 s) and draws one past a constant 2 m at once; nothing taken from the update window or the motion |
| **US 8,069,258 B1** (2007) | Electronic Arts | a local simulation of this frame from local input and a network simulation of the last (P2P) | active, about 2030-09 (fees paid; the expired US 8,678,929 above is its sibling) | Not on the path: no second, lagged simulation |
| **US 9,089,777 B2** family (2013) | Blizzard | play resumed from a reload point by replaying captured inputs | active, about 2033 (the family runs to US 11,654,356 B2, about 2033-04-09, active; its claim 1 read 2026-10-23: resuming gameplay at a prior state between checkpoints, saved states loaded and the inputs between replayed, then play begun from there) | Kept clear: Forge's replays (2026-10-23) only reproduce and check a session; nothing begins play from a recorded moment. Look again before any "play from here" feature |

Also met and outside: Microsoft US 10,924,525 (latency equalised between players), Disney US
8,676,996, King's pending continuation US 2025/0083052 (snapshot prediction), EA US 12,472,427
(lockstep), Bandai Namco US 8,734,246 (P2P input delay), Google US 11,366,586 (synthesised
inputs). Prior art: QuakeWorld and Quake 2 (prediction, the error faded, three commands a packet),
Quake 3, Tribes (2000), Bernier (2001), Source, GGPO (2006, MIT since 2019), Fiedler (2014–15),
Ford's GDC 2017 talk (the client simulating faster on the server's starvation feedback), Unity's
NetCode (2019–20: command age and time scale). Not found: any claim on pacing by buffer depth, on
"wait, never repeat", or rollback patents by fighting-game makers. Rules kept: the server reports a
measure only; the rate follows its error, in the simulation's clock alone; a late input waits,
nothing is repeated, guessed or extrapolated; no predicted state upstream; nothing about hit
volumes or places from input timing; smoothing (later) by a fixed time; redundancy by
acknowledgement; no latency equalised between players.

**Regions by load (rebalancing), searched before building it** (2026-10-23). The question: may
the layer move the strips' cuts by load, or split a busy region over two workers and merge quiet
ones? A research agent read the claims on Google Patents; claim 1 and claim 6 of US 12,681,773 were
read again here. Not legal advice: claim construction ("geographic space", "virtual location",
"processing load") is a lawyer's question.

| Patent | Holder | What the independent claims need | Status | Bearing |
|---|---|---|---|---|
| **US 12,681,773 B2** (priority 2023-06-30) | Amazon | worker-level resource consumption and per-partition information received; ownership of an identified spatial partition reassigned from one worker to another on both (cl. 6); cl. 1 adds instructions changing the number of workers | active, about 2044-03 | **The closest.** Never move identified cells, blocks or strips between workers on a load measure; strip *i* stays worker *i*; failover's target by a fixed rule (the lowest worker left), never the least loaded |
| **US 2025/0004863 A1** (2023, PCT WO 2025/006754) | Amazon | a worker splits a parent partition into children or merges them from saturation data; a simulation manager accepts | **pending** | No parent/child partitions, no split or merge events, no worker proposing them. Watch it |
| **US 11,909,601 B1** (2021) | Amazon | a node decides part of its volume goes to a newly allocated node and sends it the state | active, about 2042-04 | Forge's layer decides, workers hold nothing, no worker is allocated for a split |
| **US 12,636,578 B2** (2023) | Amazon | partitions moved between applications inside one worker by their workload | active, about 2044-02 | One strip a worker process: not on the path |
| **US 10,484,249 B1** (2015) | Amazon | a node over a load threshold re-clusters its objects around more centroids, a second node takes a cluster | active, about 2036-10 | No clustering of objects into partitions |
| **US 10,303,821 B2** (2016) | Improbable | the target chosen by load-density centres and a transfer or "tensile energy" score | active, about 2037-10 | The target is the strip an entity stands in |
| **US 10,549,189 B1**, **10,688,391 B1**, **11,229,838 B2**, **11,628,356 B2**, **11,918,893 B2**; **EP 3 897 892 B1** (2018) | Sony | users in a region over a per-compute limit, the region divided into sub-regions and compute added; or the number of computes per location adjusted from their monitored load, users' movement, user types or a prediction | active, about 2038–39 (EP: 2039, Europe) | A fixed worker pool, one worker a strip, no limits on player counts, no sub-regions, no prediction, no per-worker load in a decision |
| **US 10,001,377 B2**, **10,215,570 B2**, 9,562,775, 9,784,584 (2015) | HCL (from IBM) | regions of a **geographic space** with mobile objects; the common boundary changed by a subsystem's data processing load | active, about 2035 | Literally "move the cut by load", but described for real land, sea and air with routes. The residual risk of moving cuts during play |
| **US 11,798,119 B2** (2019) | TMRW | cells' resources from a demand ranked by the objects in avatars' fields of view; cells subdivided, then consolidated | active, about 2040-12 | No field-of-view demand, no temporary subdivision |
| **US 12,619,482 B2**, **EP 3 933 590 B1** (2020) | Hadean | each process keeps a routing tree of regions; region-addressed messages forwarded through it | active, about 2043 (EP 2040) | Entity-addressed events through the layer's flat table only |
| **US 2025/0239013 A1** (2023) | Tencent | proxies created in source and destination, synchronised, authority transferred, proxies recycled | **pending** | Ghosts are recomputed in each tick's picture, never created or recycled objects. Watch it |

Also met: Amazon US 10,230,583 (a node assuming authority before the owner acknowledges), Sony US
8,332,499 (a crossing's input buffered by a relay), Improbable US 10,025,892 (an oracle starting
workers), Linden US 8,612,196 (changes sent only when perceptible at the client's resolution, to
2028-06), Take-Two US 11,794,114 (ids renumbered when sessions merge). Prior art, public long
before: Quazal US 6,801,930 (2000, expired: region areas adjusted to equalise load), Sony US
6,370,565 (1999, expired), IBM US 7,428,588, 8,327,371 and Qurio US 8,116,323 (expired), DIS and HLA
(1993–98), Berger and Bokhari's recursive bisection (1987), Chen et al. (PPoPP 2005), De
Vleeschauwer et al. (2005), Lim et al. (2006: the boundary between servers moved), Bezerra and Geyer
(2009), BigWorld's CellAppMgr resizing cells, LAMMPS's `balance shift` (cutting planes moved to
equalise particle counts, 2013). In the US practising the prior art is no defence by itself, so
Forge stays literally outside each claim.

**Rules for rebalancing:**
- a fixed worker pool, changed by configuration, an operator or a failure only; no autoscaling, no
  worker started for a split;
- N strips for N workers, strip *i* always worker *i*, a flat list of cuts: no parent and child
  partitions, no split or merge events;
- the layer alone moves the cuts, at a tick boundary, by a fixed rule from the world it holds
  (weighted quantiles of entity positions, weights from a static cost per kind); per-worker
  timings go to the overlay, never into the decision;
- no limits on player counts, no field-of-view demand, no prediction;
- no identified cells changing owner by load;
- a cut moved hands entities over as today: at the tick's end, whole, epoch + 1, no notice, no
  buffer, no state from the old worker;
- failover's target by a fixed rule (`Layer::reassign`, the lowest worker left), never by load.

**The options:** re-cutting on a worker joining, leaving or failing is the safest and comes first.
Moving the cuts during play from entity positions is low risk under the rules, but the HCL and Sony
claims are broad: **the owner decides before it is built.** A fine grid of cells handed to workers by
load is not built.

Nothing was met on QUIC itself (an IETF standard whose participants file IPR disclosures with the
IETF; not checked here), on connect tokens, fragmentation or a link conditioner.

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
*Built (#268):* the transport as recommended, with one change: no in-process `Transport`
implementation, because the link conditioner went under quinn's socket instead, so tests run the
real QUIC stack; the token is signed with Ed25519, so servers hold only a public key; a hello on the
first stream carries the version, the channels and the token, and a refusal is a QUIC application
close with a code and a reason (`docs/demos/net.md`).

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
*Built (#268, without authority handoff yet):* as above, with entities named in a block by a
per-client slot rather than their id (a gap of a few bits instead of 20 or more for a city's
population), full states before any delta while one waits (the alternative to US 11,810,245 in
§9), a 5-bit baseline offset into the client's last 32 packets, and 2-bit size classes for the
position's and the velocity's differences. Measured on a stand-in crowd: 4.6 bytes a moving
walker's delta, 21 a full state; 720-byte snapshots at 30 Hz, about 153 deltas each; 100 clients
over QUIC at 100 ms and 2 % loss at 23.6 kB/s each with IPv4's headers for 10 minutes, under the
24 kB/s target;
4.2 ms of one core a snapshot tick for the 100 (`docs/demos/net.md`).

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
