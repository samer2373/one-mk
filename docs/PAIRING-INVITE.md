# Pairing invitations (3H)

## Use and approval

On A, run `one-mk invite --expires 10m`. Privately transfer its one-use
`onemk://pair?...` link or terminal QR to B. On B, run `one-mk join` and
paste the link. There is no automatic clipboard access or URL handler.
The positional `one-mk join '<link>'` remains supported, but warns that the
credential is exposed in process arguments (including `/proc/.../cmdline`
where readable) and possibly shell history. Prefer stdin and a private,
authenticated transfer channel.

New pairings choose an unused screen edge without moving existing
computers; running daemons agree on opposite sides using normal layout
sync. If all four sides are occupied, trust still succeeds and `status`
asks you to choose a non-overlapping placement. Re-pairing keeps the
existing placement. See [Pairing placement](PAIRING-LAYOUT.md).

After mutual proofs, B prints:

```text
Your code: 123456 (the other computer will ask you to compare it)
Trust <A's name> (<full fingerprint grouped in four-digit chunks>)? [y/N]
```

A shows B's self-asserted name, full grouped fingerprint and independently
computed code, asking `Does the other computer show 123456?` before trust.
Compare the code actually displayed on the intended B over a trusted
channel. Type `y` on both computers only if the code and host identity are
right. Anything else refuses. Prompts have a maximum two-minute answer
window; A's is additionally bounded by invitation expiry. A valid proof
consumes the link **before** either prompt or trust save. Refusal or an
interrupted exchange requires a fresh invitation. Ctrl+C before the host saves
trust cancels approval. Once the host has saved trust, Ctrl+C no longer cancels
its bounded daemon-reload and commit-acknowledgment steps: it either finishes
normally or reports that trust was saved locally but peer commit is uncertain.
It never reports an ordinary cancellation after saving trust. Process crashes,
forced termination and lost acknowledgments can still leave asymmetric trust.

The six-digit short authentication string (SAS) is neither the secret nor
a reusable password. It depends on the secret and full session transcript,
including B's certificate and the TLS connection. An impostor using a copied
link first with B's hostname produces the impostor's code, not one the real
B printed. Names alone prove nothing. Six digits offer approximately a
one-in-a-million accidental match, not 256-bit human verification. A
compromised comparison channel defeats this protection.

`--confirm` is A's default. `invite --no-confirm` explicitly skips A's
prompt and loudly says so, including the skipped comparison code. B's
`join --yes` skips its host trust prompt and is **required** for non-terminal
stdin, including scripts piping a link. B still prints its code and warning.
With both options, a stolen link used first can establish trust without any
human check. On headless hosts, stdout containing the link/QR may enter a
system journal, supervisor log, CI log or terminal recording.

Lifetimes accept seconds/minutes/hours: 1 second to 1 hour, default 10
minutes. Start sharing afterward with `one-mk run` on both computers.
`one-mk forget <id>` revokes saved trust; invitation expiry does not revoke
an already-paired device. QR rendering uses `qrcode` 0.14.1 (MIT OR
Apache-2.0), default features disabled, with an ANSI four-module quiet zone.
Terminal wrapping can damage a QR; widen the terminal or transfer the link.

## Link, transport and protocol

The strictly parsed, bounded base64url link contains a version, A's full
SHA-256 certificate fingerprint, Unix expiry, service port, fingerprint-
derived `.local.` hostname, 1–8 direct numeric addresses and a random
256-bit bearer secret. Ports must be nonzero and listener ports agree.
Unspecified, multicast, loopback, IPv4 broadcast, IPv4-mapped equivalents
and scope-less IPv6 link-local addresses are rejected. `--listen IP:PORT`
requires a usable unicast address; a host without one fails rather than
emitting a loopback link.

B tries direct addresses **before** mDNS. The fallback uses `mdns-sd`'s
local service browser, never system/unicast DNS for `.local`. Only matching
fingerprint hints in pairing mode are considered, with address filtering
and eight distinct candidates per pass. An eight-second connection budget
is shared with up to three optional busy retries; each dial is at most two seconds.
mDNS gets at most two seconds of the remaining budget. A busy refusal is
retried up to three times after independently jittered 300–900 ms waits, only
while the original connection budget permits. None of this promises availability.

TLS 1.3/QUIC uses the existing Node stack. B pins A's exact fingerprint
**inside rustls's server certificate verifier**, retaining handshake
signature checks. A mismatching server is rejected before B sends its
permanent client certificate or invitation proof. A real pinned host still
learns B's identity even if pairing is later refused.

Invitation messages on the pairing stream:

1. B sends the invitation marker, `HMAC-SHA256(secret, "one-mk invite id v1")`,
   B's length-prefixed name, service port and random 32-byte nonce.
2. A replies with availability byte `1`; unknown/unavailable IDs get refusal
   byte `0`. Only an active matching ID receives A's name, port and nonce.
3. B sends `HMAC-SHA256(secret, length("join") || "join" || transcript)`.
4. A verifies and atomically burns the invitation, computes the SAS under
   the same lock, and sends the analogous `"accept"` role MAC.
5. Both compute the `"sas"` role MAC. Its first four bytes, big-endian modulo
   1,000,000, are displayed as six digits. Both send local decision bytes.
6. Only mutual approval permits saving trust. After a successful local save,
   each sends committed byte `2` and FIN, then awaits the other's commit.
   Normal `Trusted ...` success is printed only after that acknowledgment.

The production transcript has u16-length-prefixed fields: domain
`"one-mk invitation v1"`, invitation ID, both full fingerprints, both names,
both service ports, both nonces and 32 bytes of TLS exported keying material
(same domain label; ID as context). Roles prevent reflection and the exporter
binds proofs/SAS to the exact TLS connection. The secret itself is never an
invitation message. `ring::hmac::verify` checks proofs in constant time.
This pre-merge wire revision is incompatible with earlier invitation builds;
update both computers. Existing numeric pairing is unchanged.

## State, limits and clocks

A holds the secret in a zeroizing allocation under one mutex. Exactly one
valid proof wins across racing connections. Five bad proofs for the
**correct secret-derived ID** burn the invitation. Unknown IDs never change
its guess counter or secret. Public fingerprint, advertised addresses and
estimated expiry do not reveal the ID. The original implementation derived
it from public data: the earlier documentation understated a practical LAN
attack that guessed public payload/expiry values to name and burn links.
This revision closes that route, not all denial of service.

Separate source admission allows 32 attempts per canonical source IP per
one-second bucket, with at most 256 recent buckets and eviction of expired
entries. Ordinary IPv6 neighbors have independent /128 buckets. After more
than eight distinct /128s appear in a /64 within one second, that prefix uses
a shared /64 rate budget, including attempts counted before the switch. Prefix
tracking is also bounded to 256 entries and nine retained addresses per prefix.
At capacity the oldest bucket is evicted rather than refusing every new source.
Node retains 16 concurrent
TLS handshakes / 4 per IP and a three-second handshake timeout. Invitation
headers have a 1.5-second deadline for opening the stream and sending magic plus
ID, with at most 128 short-lived header tasks and four concurrent pre-ID
attempts per source (aggregated across a /64 when its rotation limit activates).
Eight of the global slots are reserved for a source with no pre-ID attempt
open and a freshly accepted TLS connection (at most one second old at admission).
The reservation timestamp is observed when Node delivers the authenticated peer;
Node does not expose the underlying transport's handshake-completion timestamp.
Source permits are released immediately on header rejection, cancellation or a
matching ID, rather than waiting for the entire proof exchange. Only a matching secret-derived ID
can reserve one of 16 long proof slots / 4 per certificate. The complete exchange
has a ten-second transport timeout. Silent TLS clients do not reserve long slots.
Unknown-ID refusals disclose no name, nonce or proof
validity. Late joins get explicit protocol/QUIC application-close refusals,
including during approval, rather than a ten-second stream timeout.
The early application close code 3 reveals whether an invitation is still active
before checking an ID. This availability side channel is accepted; it reveals
neither the bearer secret nor proof validity. ID comparisons use constant-time
fixed-length comparison, just as MAC verification does.

Both monotonic and wall clocks bound availability. A 250 ms check detects
forward wall jumps while idle; A's approval reader also checks wall expiry.
Either clock expiring ends availability, wipes the retained link and stops
mDNS advertising, with a bounded wait for the daemon's goodbye cleanup.
A valid proof likewise clears the link and withdraws discovery before the
host prompt. B initially checks expiry, but **only A decides invitation
expiry for the final decision**, just before its approval byte. Neither
side rechecks invitation expiry after the host sends its approval: from that
point the host waits up to ten seconds for the peer's decision, independent of
invitation expiry. This closes the buffered-approval/expiry race without changing
wire bytes. Both approval orders are tested, including a joiner responding after
expiry to an already-approved host. Independent human/transport
timeouts still apply; an initially skewed B clock can reject a valid link.

## Related local hardening

On Unix, hook and `run:` action settings require a current-user-owned file.
The config path and every resolved target-parent ancestor through the filesystem
root are checked, even when a symlink points to the config folder's ancestor,
a sibling tree, or a child folder. Directories may belong to the current user
or root; group/world-writable directories require the sticky bit. File write
permissions are not exempted by a sticky bit. Existing explicit permission
trust overrides remain available. These checks do not replace OS ACL checks
or eliminate concurrent filesystem replacement races.

New peer state changes cancel the same peer's queued opposite hook, including
transitions without a configured hook. Still-pending events for that peer
cannot be overtaken by an earlier due time. Hooks and key actions have separate
eight-process budgets, so neither consumes the other's capacity.

When the Linux network watcher has never connected to the system bus, five
consecutive identical connection failures stop automatic retries entirely.
Manual `profile use` remains available; restart the daemon after fixing an
initially absent bus to rearm automatic watching. A previously working bus
continues reconnecting with the bounded 1–60 second backoff.

Windows startup removal reads the Run value through the wide-character
registry API rather than decoding `reg query` output. Non-ASCII paths remain
intact. An entry belonging to another copy is explicitly left in place; read
or delete failures are errors rather than a misleading "nothing to remove."
This path has only been cross-compiled/clippy-checked, not exercised on Windows.

## Regression verification

`cargo test -j 2 --workspace` includes paused-time peer-hook ordering and
action-budget tests, symlink permission cases, fake-signal cancellation before
and after save, adaptive IPv6 admission, reserved-slot admission, and continuous
three-source slow-loris refill while a real join completes. The refill test uses
local sockets and ephemeral identities; it does not start an input backend.
Repeat `cargo test -j 2 -p one-mk continuously_refilled_three_source_slowloris_allows_real_join -- --nocapture`
ten times when changing admission policy.

The private-bus restart test requires `dbus-daemon` on PATH. Like an
environment-dependent ignored integration test, its explicit skip condition is
an executable-not-found error; it prints `skipped: dbus-daemon missing` to stderr
(visible with `--nocapture`) and returns. Other spawn failures fail the test.
The paused-time absent-bus regression always runs, independent of that tool.

For CLI stress, run isolated host/join processes under separate throwaway
`--home` directories, leave host confirmation enabled, compare the displayed
SAS values, then approve. Production links deliberately reject loopback
addresses: use a local unicast address in the link while the three flooding
sources connect to the same listener through `127.0.0.1`. The ignored
`flood_cli_invitation_listener` helper runs six seconds of continuous refill
from `127.0.0.2`, `.3`, and `.4`; opt in with `ONE_MK_TEST_INVITE_ADDR=127.0.0.1:<pairing-port>`
and `cargo test -j 2 -p one-mk flood_cli_invitation_listener -- --ignored --nocapture`.
To exercise post-save cancellation, pause the joiner after it sends approval,
approve on the host, wait for the host's `devices.toml`, send SIGINT to the host,
and resume the joiner. Both should acknowledge and exit successfully. Always
resume/terminate all test processes and remove their throwaway homes afterward.

## Residual risks and recovery

- A link holder can race B, consume the link, or deliberately burn it.
  Default comparison/approval prevents a familiar hostname being sufficient,
  but cannot restore a consumed invitation.
- A hostile LAN can still saturate bandwidth, handshakes, buckets or pending
  slots. Devices sharing a source IP share its limit; allow one second to
  recover after a flood stops. Distributed flooding is not solved.
- Replacing the entire delivered link substitutes a different pinned host
  and secret. Neither pinning nor SAS authenticates compromised delivery or
  comparison channels. Compromised endpoints are outside the threat model.
- Saving trust is **not a distributed transaction**. Crash, disk failure,
  disconnection or lost acknowledgment between approval and commit can leave
  one side saved, or both saved with one side uncertain. On acknowledgment
  failure after local save, the command says:
  `Trust saved locally, but peer commit is uncertain. Run one-mk forget
  <peer> on both computers and create a new invitation.` Inspect `devices`,
  forget the peer on each side that has it, then use a fresh invitation.
  No automatic rollback is claimed. Concurrent trust-file edits retain
  existing last-writer behavior.
- Owned secret fields, encoded links, stdin buffers, invitation status
  objects and server control-response buffers use zeroizing wrappers/drop
  cleanup. A drops its original Invitation after creating state and wipes
  its retained link at burn/expiry; no link is saved to configuration.
  Terminal/QR renderer copies, argv, UI clients, journals, swap, allocator/
  compiler copies and crash dumps cannot be comprehensively recalled or
  erased. Secret parsing copies directly into a zeroizing destination; encoded
  links are assembled at final capacity. Stdin reads bounded bytes directly into
  a preallocated zeroizing buffer, without `read_line` growth; Unix uses an
  unbuffered `/dev/stdin` handle. Windows' standard-input runtime/OS may retain
  copies. HMAC keys are dropped immediately after their last use, but ring does
  not provide a safe zeroize API for its inner/outer pad states. Those states,
  compiler-created stack/clone temporaries and transport buffers are not
  guaranteed wiped. This is not a complete process-memory-erasure guarantee.
- The private control hook returns `invitation\t<expiry>\t<link>\n` only
  while active, otherwise `none\n`. A sharing daemon forwards it to the
  invitation endpoint under `<home>/invitation`. Only endpoint/lock metadata,
  not the link, goes on disk. Same-user processes can access this and identity
  files and are inside the local trust boundary. Existing Unix permissions
  and Windows identity ACL limitations remain unchanged.

## Validation

Public-ID dependency and invalid-address regressions were run against the
original implementation first and failed. Tests now call production
transcript construction and independently change ID, both fingerprints,
names, ports and nonces; role separation and replay over a second TLS
exporter are checked. SAS agreement, secret/certificate dependence, and a
copied-link/same-hostname impostor are covered. The impostor regression fixes
both nonces, names, ports, ID, secret and TLS exporter, varying only the peer's
certificate fingerprint. It checks both proof and SAS changes; an already-used
invitation never admits a second real join.

A wire-level loopback test sends 1,000 wrong-ID streams plus 1,000 wrong-proof
with wrong-ID streams, then successfully performs the real proof/approval on
the still-active invitation. A unit test checks state directly. TLS tests
inspect the server's certificate state during an in-memory handshake: a
rejected pin discloses no client certificate. Other tests cover source
limits, strict parsing, clocks, single-use races, refusal, actual persistence,
commit acknowledgment loss, stdin refusal without `--yes` and argv warnings.

Reproducible Linux PTY/process smoke:

```sh
cargo test -j 2 --workspace
cargo clippy -j 2 --workspace --all-targets -- -D warnings
cargo build -j 2 -p one-mk
cargo build -j 2 -p one-mk-net --example invite_probe
python3 scripts/test-invite-security.py
```

The script uses only `/tmp` homes and invite/join/devices commands, checks
the UI hook, starts a public-data-only hostile process, compares actual PTY
codes before approving, and tests scripts, refusal, late joins and expiry.
Every started process is cleaned up. Since link loopback literals are
rejected, the process test uses a usable local unicast address (locally
routed on the same computer); Rust wire tests use `127.0.0.1` sockets.
No live input backend, normal user config, `cargo fmt` or fat-LTO build is
used. Physical QR scanning, multi-machine discovery and Windows/macOS runtime
behavior remain unverified. Compile checks are not runtime validation.

### Recorded validation (2026-10-29)

`cargo test -j 2 --workspace`: **596 passed, 5 existing ignored, 0 failed**.
Workspace clippy and the following target checks pass with `-- -D warnings`:

```sh
cargo clippy -j 2 --workspace --all-targets -- -D warnings
cargo clippy -j 2 --target x86_64-pc-windows-msvc -p one-mk-input -p one-mk-proto -p one-mk-ctl --all-targets -- -D warnings
cargo clippy -j 2 --target aarch64-apple-darwin -p one-mk-input -p one-mk-proto -p one-mk-ctl --all-targets -- -D warnings
```

On this Linux host, ring's cross-target C compilation used temporary Clang
21/LLVM tools and genuine Zig 0.13.0 libc headers, not an installed platform
SDK. `CC_<target>`/`AR_<target>` selected those tools; `CFLAGS_<target>` used
`-nostdlibinc -isystem <headers>/any-macos-any` or `any-windows-any`.
The Windows C flags additionally used `-fgnuc-version=4.2.1` so MinGW headers
select GNU-style declaration attributes accepted by Clang's MSVC target.
The Rust targets remained exactly those shown above. No target cfg override,
Rust warning suppression, dependency patch or fabricated header was used.
The full Windows binary and its tests also pass `cargo check -j 2 --target
x86_64-pc-windows-msvc -p one-mk --all-targets`, including the new batch-hook
environment handling. That additional check reports existing dead-code warnings
in doctor, netwatch and notify; full-binary Windows strict clippy is not green
because of those unrelated warnings. The required three-crate target checks
above are warning-free. Windows behavior has only been compile-checked.

Failing-first regressions reproduced unsafe POSIX shell conversion, stale hook
queues, cross-peer coalescing, unsafe symlink target folders, legacy clipboard
fetch retention, unbounded alignment, slow invitation headers, source-bucket
lockout and IPv6 rotation. The expiry/persistence regression also failed when
the old expiry-bound approval timeout was temporarily restored, then passed
with the fix. Lane independence, descendant cancellation and corrected
production-path/transcript tests provide additional coverage; they are not
claimed as independent failing-first demonstrations. The shell round-trip test
executes 441 hostile word combinations through `/bin/sh`.

Captured real-process output (invitation bearer links deliberately omitted):

```text
Same-host unicast endpoint: 192.168.1.18 (loopback link literals intentionally rejected)
Slow-loris: holding 20 silent TLS connections across 5 certificates; no magic, ID or secret supplied.
Slow-loris pairing: both PTYs show 001382; real join succeeded and both production stores saved.
Approval host-first: matching SAS 276393; both saved after expiry crossed following host approval.
Approval joiner-first: matching SAS 225465; both saved.
Ctrl+C during approval: host exits nonzero; neither saved.
Public-only attacker: 1000 wrong-ID + 1000 wrong-proof/wrong-ID attempts; 192 explicitly refused, 1808 source-rate-limited; no secret supplied.
Late join: explicit 'invitation already used or expired'; active UI link cleared.
B: Your code: 996050 (the other computer will ask you to compare it)
A: Does the other computer show 996050? Trust it with your keyboard, mouse and shared clipboard? [y/N] -> y
B: Trust samer-Victus (631b 96d2 929b d7ca 6499 d211 df80 dc7b 2352 4002 0f31 644a 5e95 6239 ba6e 48ae)? [y/N] -> y
A devices:
NAME                     ID         SIDE    ADDRESS
samer-Victus             9b1f-67f0  left    192.168.1.18:24900
B devices:
NAME                     ID         SIDE    ADDRESS
samer-Victus             631b-96d2  right   192.168.1.18:24900
Scripted --yes / --no-confirm: both warnings printed; both devices saved.
Joiner refusal: neither trust store created.
Expiry during approval: both commands fail; neither trust store created.
All smoke checks passed; all started processes exited or are stopped in cleanup.
```
