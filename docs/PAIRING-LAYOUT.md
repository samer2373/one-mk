# Placement after pairing (QA M9)

`one-mk pair` (including the window's `pair --machine` subprocess) and
`invite` / `join` use the same placement selection and persistence code.
New computers take the first unused side in **Left, Right, Top, Bottom**
order. Offline paired computers reserve their sides too. Re-pairing keeps
the existing side, offset, placement origin and Ctrl/Cmd setting; only the
name and addresses are refreshed. Other paired computers are never moved.

## Agreement between computers

The two local selections are provisional until the daemons connect. The
existing `Control::Layout` message communicates the chosen placement as
`side_of_you = side.opposite()`. Its version comparison and fingerprint
tie-break are unchanged; no pairing or control wire format was extended.

Automatic placements use small positive versions: two per occupied side,
plus 1, plus 1 for the numeric-pairing connection initiator (or invitation
host). This gives the computer with more occupied sides priority over an
empty newcomer. With equally occupied layouts, the numeric initiator or
invitation host wins. The other daemon adopts the opposite side. A legacy
peer's version-zero default also adopts this newer placement.

If the winning automatic proposal would overlap another local computer,
the receiving daemon proposes its next available side under a newer
version, still using `Control::Layout`. These retries are bounded by
automatic version 16. If no mirrored free edge can be found, placement
is left pending rather than moving an existing computer or silently
overlapping it. A re-paired existing record is protected above the automatic
range, retaining any already-higher user version. Explicit layout edits
continue to use the normal millisecond-based versions.

Automatic placements are saved with `placed_by = "pairing"`, so their
nonzero synchronization versions do not disable push-to-place learning.
Existing user, learned and confirmed placements retain their origin.
Layout received from the other computer remains a choice made there,
as before.

## When all four sides are occupied

Trust still succeeds. The full computer prints a one-line **placement
needed** message; `status` repeats it, even while the daemon is stopped,
and `devices` says `placement needed` instead of pretending a side was
selected. The window also receives the usual paired event and can show
the daemon's layout warning. The extra `placement-needed` machine event
does not change the window's existing paired-event format.

Pending records are persisted as `placed_by = "pending"`. Their internal
fallback side is not armed for crossing, reserved for wrapping, used for
automatic switching, or included in broadcast targets. An explicit switch
to one fails immediately with placement guidance. Routine layout messages
and re-pairing do not clear the pending state. Choose a non-overlapping
placement, for example:

```sh
one-mk layout '<computer name or id>' right --offset 1920,1080
```

Use coordinates matching your actual monitors, or place the computer in
the window's Layout tab. A local or remote explicit layout edit clears the
pending state and is mirrored normally. A peer with a free edge may still
have its own provisional side while this computer awaits placement; there
is deliberately no new wire representation for an unplaced computer.

New builds interoperate with old peers through the existing layout
message and version rules. Downgrading the full computer to a build that
does not understand `placed_by = "pending"` loses pending-state protection;
choose its placement before downgrading.

## Regression coverage

`crates/one-mk/tests/pairing_layout_cli.rs` launches real daemons and pairing
commands with empty environments, private homes/runtime directories,
ephemeral ports, the null backend, discovery disabled and notifications
disabled. Numeric pairing uses loopback; invitations require a usable
non-loopback local address by design. Every child is killed and reaped.

Coverage includes two- and three-daemon numeric and invitation flows,
both roles for the already-paired computer, mirrored sides, no overlap
warnings, re-pairing with an explicit offset, both endpoints having
occupied sides, all four sides full, persistence of pending placement
through re-pairing, and resolving it with an explicit layout. Unit tests
also cover selection, version priorities, preserving settings and marker
serialization. The original numeric defaults fail the third-computer and
full-layout checks; the original invitation defaults fail the
third-computer check as well.

Validation on Linux (2026-10-31): three consecutive clean-environment
`cargo test --offline -j 2 --workspace` runs each passed 1,149 tests
(14 ignored), including all five real-daemon placement regressions.
Strict `cargo clippy --offline -j 2 --workspace --all-targets -- -D warnings`
passed. Cross-target strict Clippy also passed for
`x86_64-pc-windows-msvc` and `aarch64-apple-darwin`, with
`-p one-mk-input -p one-mk-proto -p one-mk-ctl -p one-mk-app -p one-mk-tray
--all-targets`. These cross-target checks do not verify Windows/macOS
runtime behavior.
