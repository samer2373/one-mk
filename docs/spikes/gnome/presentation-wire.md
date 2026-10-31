# Presentation payload and authenticated session (6D-M1 step A, laboratory)

October 31, 2026. This document describes `one_mk_wall_sync::presentation_wire`
(the bounded plan payload) and `one_mk_wall_sync::presentation_auth` (the
authenticated daemon-to-renderer session that carries it). Since step A the
laboratory daemon and renderer call them for real; see
[the step A evidence](m1-presentation-evidence/README.md). Both ends are compiled
only with laboratory features (`one-mk/wallpaper-m1-lab`, `one-mk-wall/m1-lab`)
that release builds never enable. **Production authorization stays false**:
plain `one-mk-wall --present=gnome` still refuses, and nothing here selects media,
decodes, or creates a surface.

Laboratory features fail compilation when debug assertions are disabled, including
the normal release profile. The artifact checker also rejects activated daemon
or renderer laboratory features anywhere in its supplied resolved Cargo metadata.
Metadata and feature-tree inputs must describe the actual build; this is not
binary attestation or protection against a deliberately modified build profile.

## Version 1 encoding

All integers use little endian. No alignment padding, paths, credentials, optional
extensions or trailing bytes are allowed. The total payload is at most 4096 bytes;
the session reply envelope enforces its own overhead-inclusive 4352-byte body
budget before deserializing (this codec's limit alone does not bound a D-Bus message).

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

## Gate boundary

`accept_encoded(caller, bytes, enabled, live_revision)` compares the independently
pinned caller identity, reconciles the independent live owner policy, structurally
decodes the bounded payload and calls the semantic admission gate. A refused plan
(wrong caller, malformed, oversize, stale, replayed, wrong revision/generation,
invalid geometry) **leaves the admitted plan in place** and never lowers the
accepted sequence, so a replay cannot restore an older plan. Disabled, removed or
replaced consent and owner loss permanently revoke the gate instance; a layout or
suspend change clears admitted intent until a fresh plan arrives. The Gate performs
no I/O; the renderer must destroy presentation resources when intent is absent.

## Authenticated session (`presentation_auth`)

Trust is never ambient: being on the bus, owning a name or serving the object path
grants nothing.

1. **Record.** On start the laboratory daemon draws a fresh 32-byte key and 16-byte
   authority generation and writes `WAUT`, version 1, key, generation, its PID and
   its unique bus name (bounded to 512 bytes) to
   `$XDG_RUNTIME_DIR/one-mk-wallpaper-m1/authority` (directory 0700, file 0600,
   both owned by the user, written through a staging file and `rename`, guarded
   by an exclusive `flock`). A clean exit removes the record only if it is still
   its own generation. SIGKILL leaves a stale record whose name has no owner.
2. **Renderer admission of the authority.** The Shell-spawned renderer first
   authenticates the Shell bridge (Wayland peer credentials, Shell-owned bus
   name, `Hello`, initial `PauseHint`). It then opens the record without following
   symlinks, requires private ownership/modes, subscribes to `NameOwnerChanged`
   for the record's unique name and only then checks that this name is owned by
   the record's PID and the same UID.
3. **Per-request proof.** Each `org.onemk.WallpaperAuthority1.Plan(ay nonce,
   u layout_serial, a(iiiid) monitors, ay tag)` carries a fresh 32-byte nonce and
   a keyed BLAKE3 tag over a domain string, both unique names, the nonce, serial
   and monitors. The daemon answers only callers whose tag verifies (constant-time
   comparison) and whose UID matches; unauthenticated callers get `AccessDenied`
   from a separate budget, and at most one live renderer is pinned. The reply
   `(b enabled, t revision, ay plan, ay tag)` is tagged over both names, the
   renderer's nonce and every reply field, so a reply cannot be replayed into
   another request or reflected to another connection.
4. **Live consent.** The daemon derives `(enabled, revision)` from the `[wallpaper]`
   settings it applies (`one-mk wallpaper consent revoke|grant`, `wallpaper
   disable|enable`, any reload) and sends an empty unicast `Changed` signal to the
   pinned renderer on every change; the renderer then fetches immediately (a
   2-second heartbeat fetch is only a safety net; owner loss is observed directly).
   Each applied consent transition also advances an internal epoch. A pinned
   connection from an older epoch receives a revoked reply even if a disable and
   re-enable (or revision replacement and restoration) were coalesced before its
   next fetch. A new renderer connection may use the latest grant. Epoch
   exhaustion permanently disables this authority's consent until restart.
5. **Teardown.** The renderer ends the presentation and exits on: a reply with an
   invalid tag, an `AccessDenied` from the authority, a request unanswered within
   750 ms, consent disabled/removed/replaced (`Revoked`), loss of the authority's
   connection (exit, crash, SIGKILL), 16 refused plans in a row, or an invalid
   layout/pause from the Shell bridge. Oversize (>4352-byte) or mistyped reply
   envelopes are refused without being deserialized and keep the admitted plan
   (the bus itself still delivers up to its own message limit).

The plan's per-monitor slices are this computer's monitors translated by its
origin on the shared canvas; the canvas itself, including reserved space for absent
computers, is frozen per session. Laboratory-only directives (SIGUSR1 plus a file
in the daemon's home) pause/play or inject one fault (replay, stale sequence or
revision, oversize payload or envelope, malformed reserved bits, forged tag) so the
renderer's refusals can be exercised with real processes.

**Threat-model limits.** Same-user code that can read the record or ptrace either
process is not defended against (as for any same-UID desktop service); the record
does keep sandboxed same-UID bus clients that cannot read the runtime directory out.
The authority, record path and directives are laboratory artifacts, not a shipped
interface; the production bootstrap (who launches the renderer with which
credential, packaging, AppArmor/portal story) is still open.

## Failure-path tests

`cargo test --offline -j 2 -p one-mk-wall-sync --test presentation_wire` exercises
ten codec cases (round trips, truncation, trailing data, version/flags, counts,
UTF-8, budgets, malformed-intent retention, live consent/identity, owner loss,
all single-byte mutations). `--test presentation_auth` adds eleven session cases
(record bounds/mutations and key redaction, tag binding of every field and both
directions, publisher sequencing/slicing, admission, forged/stale/wrong-key/other
connection teardown, refusal with retained plan for stale/replayed/oversize/
malformed/foreign-generation plans, first-plan refusal without pinning, consent
revocation/replacement and owner loss, refusal budget, layout/pause freshness).
Real-process proof is the harness run, not these unit tests.
