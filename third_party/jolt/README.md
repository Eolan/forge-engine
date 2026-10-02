# Jolt Physics, vendored

[Jolt Physics](https://github.com/jrouwe/JoltPhysics) by Jorrit Rouwé, **v5.6.0**
(commit `e77f175595e64cb44218cc9d9d56fc365ad0e36a`): its library folder `Jolt/` and its
`LICENSE` (MIT), unchanged. The samples, tests, viewer and build scripts are left out.

`crates/forge-physics/build.rs` compiles it with `cc`, with the defines and flags of Jolt's
CMake for `CROSS_PLATFORM_DETERMINISTIC` and `DOUBLE_PRECISION` (D-009, issue #136).

To move to a new release: replace `Jolt/` and `LICENSE` with the release's, update the version
and commit above, check `build.rs`' list of left-out folders against the release's
`Jolt/Jolt.cmake`, and run `cargo test --release -p forge-physics`. A release may change the
simulation's results: `PILE_HASH_300` in `crates/forge-physics/src/tests.rs` then takes the new
value, on Windows and Linux alike, and the commit says so.
