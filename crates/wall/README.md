# one-mk-wall-sync

Experimental I/O-free synchronization primitives, licensed MIT OR Apache-2.0
like the workspace. No system-clock writes, sockets, decoder, heartbeat or
wallpaper integration exists in the library. Linux-only development dependencies
are used by the real-transport example, not by the core or foreign-target checks.

## Integration contract

1. Authenticate and authorize the coordinator and a committed scene. Assign a
   connection-local `session: u64` handle to that complete manifest. Never treat
   a session handle or timestamp as authentication. A generation fences the
   authority/monotonic time domain. Keep outstanding `(session,generation,seq,t1)`
   requests and match replies before calling `Estimator::observe`.
2. Take eight timestamp exchanges about 20 ms apart and pass the completed
   burst, at most 64 responses. Do not pool arbitrary unsolicited responses.
   Require three distinct good replies. Within-burst reorder is accepted;
   completed older bursts are deliberately discarded. Retry missing bursts.
3. Construct `Timeline` from committed coordinator-monotonic epoch, loop
   duration and media start. Start after pre-roll at a future epoch. An estimator
   has no mapping until a burst succeeds. Future epochs yield `Action::Wait`.
4. Call `Controller::tick` from the renderer's clock. Use **presented** PTS,
   not last decoded PTS. `Wait` means retain the previous rate. `OnTime` means
   set rate to 1.0; rate adjustments are absolute, not multiplicative deltas.
   Seek/hard-resync also reset adapter rate to 1.0. Adapter applies crop changes
   and pre-roll atomically; this library cannot guarantee flash-free seeking.
5. Select latest eligible **unwrapped** frame PTS with `frame_for_target`;
   drop earlier entries and retain future ones. A slow decoder must repeat a
   recent frame, request a smaller representation, or show a poster, never
   accumulate a FIFO backlog or slow the shared timeline. At most 256 entries
   accepted by this helper; the product should normally queue 2–3 frames.
6. On sleep, reboot, backwards local time, group pause, or explicit coordinator
   takeover, fence old replies, reset estimator and controller, establish a new
   generation/epoch and fresh burst, then pre-roll to current PTS. Merely losing
   the network does **not** reset playback: the last affine map free-runs and
   uncertainty grows. Keep the frozen manifest until an explicit authorized
   takeover. Never infer a new leader from missing packets.

Library failures do not mutate estimator/controller state, except successful
ticks and observations. Untrusted non-finite, negative, implausibly large or
badly ordered times return errors. Arbitrary first offsets up to one day are
necessarily admissible: a consistent liar and persistent one-way delay are
not distinguishable from a genuine offset. Even a low uncertainty estimate
cannot establish truth or prove physical synchronization. Bounds are spike
policy, not a hardened time-distribution security protocol.

## Measurement tools

`cargo run -j 2 -p one-mk-wall-sync --example simulate` runs 1,152 deterministic
one-hour cases and emits CSV. For speed without release/LTO, use
`cargo rustc -j 2 -p one-mk-wall-sync --example simulate -- -C opt-level=2`, then
`target/debug/examples/simulate`. The library stays in the development profile.

`cargo run -j 2 -p one-mk-wall-sync --example transport` launches a second
process, pairs ephemeral `one-mk-net` nodes, places a UDP delay proxy in their
real QUIC path and exchanges the independent fixed-size experimental envelope.
It uses Linux CLOCK_MONOTONIC only for a known cross-process ground truth.
Other target builds deliberately stub this measurement, not the pure core.

See `docs/spikes/wallpaper-sync-spike.md` for results, limitations, wire layout,
reproduction, decoder licensing and next release gates. These examples do not
change the existing codec, protocol version, daemon behavior, or user settings.
