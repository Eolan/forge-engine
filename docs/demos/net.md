# Net — the transport, its protocol and the link conditioner (`forge-net`)

Phase 5's first step (#268, D-010, D-062's parallel sessions): the networking, built and tested
headless on loopback, never touching the GPU or a window. A server and its clients talk QUIC
through a link conditioner that loses, delays, duplicates and reorders their packets from a seed.
Everything binds 127.0.0.1.

```
cargo test --release -p forge-net
cargo run --release -p forge-net --bin net-loopback -- --clients 8 --seconds 10
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

`cargo test --release -p forge-net`: 14 unit tests (the conditioner's rates, order and
queue; fragments in any order; tokens; the handshake's bytes) and 10 on loopback, 12 s in all.

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

## Next

- Replication on these channels (step 2): snapshots with deltas against an acknowledged
  baseline, quantisation, a priority accumulator under a byte budget, interest by cells.
- A handle for the simulation's thread (the engine's network thread runs the tokio runtime;
  the simulation hands messages over and never waits on I/O).
- The server PC and the Raspberry Pi come later, on the owner's word: nothing here binds
  anything but loopback.
