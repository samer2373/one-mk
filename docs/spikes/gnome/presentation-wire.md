# Presentation payload precursor — not daemon IPC

October 31, 2026. This document describes `one_mk_wall_sync::presentation_wire`.
It is an I/O-free codec, **not** an exported D-Bus interface, daemon service,
authorization bootstrap, media selection command, decoder or renderer.
`Gate::accept_encoded` is an admission entry point for future transport callers;
no production daemon or renderer calls it yet. Stage A remains **not reached**.

## Version 1 encoding

All integers use little endian. No alignment padding, paths, credentials, optional
extensions or trailing bytes are allowed. The total payload is at most 4096 bytes;
the future D-Bus envelope must enforce its own overhead-inclusive body budget
before deserializing. This codec's payload limit does not bound a D-Bus message.

| Offset | Bytes | Field |
| --- | --- | --- |
| 0 | 4 | `WPLN` magic |
| 4 | 1 | Version, exactly 1 |
| 5 | 1 | Flags: bit 0 looping, bit 1 manually paused; other bits zero |
| 6 | 2 | Reserved, zero |
| 8 | 16 | Authority generation |
| 24 | 8 | Consent revision |
| 32 | 8 | Monotonic sequence |
| 40 | 4 | Bridge layout serial |
| 44 | 32 | Imported media content hash |
| 76 | 16 | Frozen canvas rectangle |
| 92 | 2 | Slice count, at most 16 |
| 94 | variable | Ordered slice records |

A rectangle is `x:i32, y:i32, width:u32, height:u32`. Each slice record contains
`screen_length:u16`, that many UTF-8 bytes, then its canvas rectangle. Screen names
must contain 1–256 bytes and no Unicode control characters. The total byte budget
can be exhausted before the per-screen/count limits; the encoder rejects such
plans without constructing an oversized buffer. Decoder allocations are bounded
by the validated count and screen length, never untrusted capacity hints.

Structural decoding does **not** confer admission. Zero hashes, invalid geometry,
duplicate or overlapping slices, stale sequences, changed canvas and mismatched
generation/revision/serial still pass through the existing semantic Gate. The
frozen canvas is preserved verbatim; missing/asleep computer space is not removed.

## Gate boundary

`accept_encoded(caller, bytes, enabled, live_revision)` first clears current intent
and compares the independently pinned caller identity. It then reconciles the
independent live owner policy, structurally decodes the bounded payload and calls
the existing semantic admission gate. A malformed payload clears current intent
but does not reset the accepted sequence; replay cannot restore it. Disabled,
removed or replaced consent permanently revokes this gate instance. An incorrect
caller cannot replace its policy snapshot or authority. The caller must tear down
decoders/surfaces whenever admitted intent is absent; the Gate performs no I/O.

The API deliberately accepts no credentials inside the payload. Future integration
must obtain the unique bus name, UID and PID independently, establish a trusted
real-daemon bootstrap, subscribe before taking initial snapshots, reconcile live
consent independently of each plan, authenticate the inherited Wayland connection,
and observe daemon owner loss even when no new plan arrives. It must terminate
the renderer and tear down surfaces on revocation, daemon loss and protocol faults.
Those guarantees are **not implemented or proven** by this codec or its tests.

## Failure-path tests

`cargo test --offline -j 2 -p one-mk-wall-sync --test presentation_wire` exercises
ten cases: exact round trips/flags/reserved canvas; every truncation and trailing
data; version/magic/reserved bits; counts and lengths; UTF-8/control characters;
total encoder/decoder budget; malformed-intent teardown with retained replay
floor; independent live consent and identity; semantic consent and terminal owner
loss; arbitrary byte lengths and all single-byte mutations of a valid payload.
These are **unit tests**, not bus, process, media or compositor evidence.
