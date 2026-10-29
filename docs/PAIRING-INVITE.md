# Pairing invitations (3H)

`one-mk invite --expires 10m` on A prints a one-use `onemk://pair?...`
link and ANSI QR code. Transfer the link privately to B, then run
`one-mk join '<link>'` there, or run `one-mk join` and paste one line on
stdin to avoid putting the credential in shell history/process arguments.
There is no automatic clipboard access or OS URL-handler registration.

A prints **`>>> B joined: certificate <full SHA-256 fingerprint> <<<`**
after a valid proof. By default (`--confirm`), A must type `y` before either
side trusts the other. Anything else refuses. This can be done over an
existing remote terminal; no code comparison at the two screens is needed.
For unattended A, explicitly use `--no-confirm`: the first valid joiner is
then trusted automatically. That is a significant security choice.
Names are sanitized but self-asserted; an attacker can name their device B.
A full fingerprint identifies the device, but is only useful for approval
if the operator already knows which fingerprint to expect.

The lifetime defaults to 10 minutes, accepts seconds/minutes/hours, and is
limited to 1 second–1 hour. Confirmation has a 120-second timeout and must
finish before expiry. Closing the invitation process, refusing, or losing
the connection after a valid proof consumes the invitation. Generate a new
one to retry. Invitations **die with the process** and never survive a
restart, including restarting the invitation process after a daemon restart.
The sharing daemon can restart independently while `invite` remains open.

## Link and wire protocol

The link is `onemk://pair?v=1&d=<URL-safe base64 without padding>`. Its
bounded binary payload is:

1. Version byte `1`, A's 32-byte SHA-256 certificate fingerprint, absolute
   expiry (u64 Unix seconds, big endian), A's sharing service port (u16).
2. Length-prefixed mDNS host (`one-mk-<short fingerprint>.local.`), address
   count (1–8), and length-prefixed IP:pairing-port addresses. All lengths
   are u16 big endian. All addresses must use the same nonzero pairing port.
3. A cryptographically random 32-byte (256-bit) secret.

The decoder bounds the URL at 1600 ASCII bytes, validates lengths, address
count, ports, mDNS host, versions, UTF-8, and trailing/truncated data. The
mDNS name and addresses are routing hints. They never establish identity.
A's certificate is pinned to the fingerprint from the link **before B
opens a pairing stream or sends a proof**. Mutual TLS verifies possession
of each certificate's private key, as in normal pairing.

The exchange uses the existing QUIC `StreamKind::Pairing`, with a distinct
invitation marker `ff ff 4f 4d 4b 49 01`. No `PROTOCOL_VERSION` or ALPN
change is needed. Old numeric-pairing code rejects the initial `ffff` as an
oversized name, and invitation code rejects old numeric-pairing messages.
There is no downgrade or automatic fallback to a different pairing mode.
The existing commit/reveal numeric pairing implementation is unchanged.

Messages, in order (A is always the invitation responder):

1. **B → A:** marker; invitation ID `SHA256(public link payload excluding
   secret)`; B's length-prefixed name, service port, random 32-byte nonce Nb.
2. **A → B:** A's length-prefixed name, service port, random 32-byte nonce Na.
3. **B → A:** `HMAC-SHA256(secret, length("join") || "join" || transcript)`.
4. **A → B:** after atomic check-and-burn,
   `HMAC-SHA256(secret, length("accept") || "accept" || transcript)`.
5. **Both → other:** one byte local decision (1=yes, 0=no), then stream FIN.
   Only two yes decisions permit saving trust. A's decision follows its
   prompt unless `--no-confirm` was explicitly requested.

The transcript is a canonical series of u16-length-prefixed fields:
`"one-mk invitation v1"`, invitation ID, A fingerprint, B fingerprint,
A name, B name, A service port, B service port, Na, Nb, and 32 bytes of TLS
exported keying material (label `"one-mk invitation v1"`, context invitation
ID). Fields and roles cannot be concatenated ambiguously or reflected.
Names/ports are bound before approval. The TLS exporter additionally binds
proofs to the exact connection, even if nonces were accidentally repeated.

**The secret is never a network message**, including inside TLS: only keyed
MACs are transmitted. Without the 256-bit key, a network observer cannot
compute a proof from its transcript. A uses `ring::hmac::verify`, which
compares the MAC in constant time; B uses the same verifier for A's reply.
The server never compares secrets or MACs using ordinary equality.

## State, limits, and idle cost

A stores a secret in a zeroizing allocation behind one mutex. Valid proof
verification and consumption happen under that mutex, before any reply or
approval prompt. Concurrent valid proofs therefore have exactly one winner.
Five bad MACs for the active invitation burn it, even across certificates
and IP addresses. Malformed handshakes and unknown invitation IDs do not
exercise the MAC oracle; they are rejected under the admission/time limits.
Refusal never restores a used secret. Expiry checks use both wall time and a
monotonic deadline so moving A's clock backward cannot prolong the link.
B also checks its wall clock; a badly skewed clock can reject a valid link.

This is a temporary pairing listener using the same `Node::bind_any` /
`Node::accept` machinery as `one-mk pair`, not a second TLS stack or a bypass
on the sharing daemon. Node's existing limits remain: 16 concurrent/queued
TLS handshakes, 4 handshakes per IP, a 3-second handshake timeout. Invitation
attempts additionally reuse `app::Pending`: 16 total, 4 per certificate,
with permits held through result delivery and a 10-second exchange timeout.
The state is checked again when a proof arrives. Accepted peer connections
are closed on failure or process completion. QUIC transport stream limits
remain in effect. No polling timer is introduced; the idle listener waits
on accepts, the private control channel, Ctrl+C, and a single expiry timer.
mDNS retains the existing discovery behavior when enabled.

Secrets are not saved to configuration, trust files, status logs, or the
normal status response. The printed link/QR and temporary UI response are
necessarily copies of the credential; terminal scrollback, crash dumps,
swap, and same-user processes are outside the memory-erasure guarantee.
The invitation state and decoded secret buffers use `zeroize`; this is not
a claim that every displayed or encoded copy can be erased.

## Threat model

- **Shoulder surfing / QR photography / chat history:** anyone who reads
  the link has the complete bearer credential. They can join once **if they
  use it first**, subject to A's default approval. Delete shared copies and
  prefer a private authenticated transfer channel. Expiry limits exposure,
  but does not make posting an active link publicly safe.
- **Intercepted before use:** stealing the original link allows racing B.
  With `--no-confirm`, the winner gets full paired-device privileges. With
  confirmation, A can refuse, but a familiar-looking name is not proof of
  the intended person's identity. If the attacker can replace the entire
  out-of-band link, they can substitute their own server identity and
  secret; the protocol cannot authenticate a compromised delivery channel.
- **Hostile LAN race:** a LAN device without the secret cannot produce a
  valid proof or acquire trust. It can exhaust handshake/attempt slots or,
  if it knows the invitation ID, send five wrong proofs to burn the link.
  This deliberate fail-closed denial of service is preferable to an
  unlimited guessing oracle. The limits bound resources, not availability
  on an actively hostile network.
- **Replay / opening twice:** a captured proof is tied to both TLS
  certificates, names, ports, nonces, and this TLS connection. Replaying it
  on another connection fails. Reusing the whole link after valid use,
  refusal, or expiry fails. The same legitimate B must obtain a fresh link
  after an interrupted attempt that already proved knowledge.
- **MITM / forged mDNS / substituted address:** TLS proves possession of the
  certificate private keys, and B pins A to the out-of-band fingerprint.
  A fake server with another certificate is rejected before receiving a
  proof. Relay/modified proofs fail the transcript and TLS exporter MAC.
  A packet relay can forward encrypted traffic but cannot change identities
  or obtain the secret. Observers may learn IPs and traffic timing.
- **What a link holder cannot do:** merely reading the link does not reveal
  A's TLS private key, impersonate A's certificate, decrypt other sessions,
  pair multiple devices with one invitation, or reuse it after expiry/use.
  A device that wins and is trusted keeps normal paired-device access
  after the invitation expires; revoke it with `one-mk forget <id>`.
- **Local access:** same-user code can read the private control socket and
  identity files and is already within the trust boundary. Unix uses a
  0700 directory and 0600 control socket; other systems use the existing
  authenticated loopback control transport and per-user directory policy.
  Windows identity storage's existing ACL limitation remains unchanged.

Trust saving, like numeric pairing, is not a distributed transaction. A
crash, disk failure, or connection loss around final confirmation can leave
only one device saved. The invitation remains burned; inspect `devices`,
forget any unintended entry, and create a new invitation. Concurrent edits
to `devices.toml` have the existing trust-store last-writer behavior. The
invitation listener is single-instance per home via a local file lock.

## Control hook and integration

`one_mk_ctl::client::Request::Invitation` encodes/parses `invitation` and
returns `invitation\t<Unix expiry>\t<link>\n`, or `none\n`. The additive
`InvitationStatus` codec handles this separately from ordinary `Status`.
A sharing daemon forwards the request asynchronously to the active invite
process's private control endpoint under `<home>/invitation`; a UI can query
that endpoint directly when no daemon runs. Only endpoint/lock metadata is
on disk, never the link. No app/tray edits are needed. After consumption,
including while A's approval is pending, the hook immediately returns none.
After process exit or expiry it cannot recover an invitation.

`--listen IP:PORT` and `--no-discovery` support isolated/testing networks.
The default pairing port is ephemeral. `--service-port` on each command
specifies that computer's later `run` port (default 24900); the MAC binds it
and both trust stores remember it. Discovery and a best-effort bounded
mDNS lookup are only fallback hints when direct addresses have changed.

QR encoding uses `qrcode` 0.14.1 (MIT OR Apache-2.0), a small, established,
passively maintained Rust encoder. All default features are disabled:
no image library and no transitive dependencies. The CLI draws black/white
ANSI half blocks with a four-module quiet zone. Terminal wrapping can damage
a large QR: widen the terminal or use the printed link. QR scanning on a
phone does not install a URL handler or replace the `join` command.

## Validation

Unit tests cover strict link round trips, truncations at every byte,
oversized/non-ASCII/malformed and wrong-version links; wall and monotonic
expiry; single use; five-guess burn; role/transcript MAC binding; concurrent
valid proof races; and replay across two real TLS connections even with
identical nonces. Network integration tests exercise two nodes and persisted
trust without human input, wrong secret, expiry, reuse, a different client
certificate, an impersonated server certificate (no stream opened), refusal,
and five wrong guesses followed by the correct secret. Existing numeric
pairing loopback tests continue to run unchanged.

The implementation report records workspace tests, clippy, cross-target
checks and a real two-process invitation / two-daemon null-backend smoke.
Windows and macOS are compile-checked only, not tested on real hardware.

### Recorded implementation validation (2026-10-29)

Branch: `t3/implement-pair-invite-flow`, based on `311dc81` (the next
commit in the baseline history is `55c8b00 Plan: wave 4...`).

- `cargo test -j 3 --workspace --offline`: **402 passed, 3 existing ignored**.
- `cargo clippy -j 3 --workspace --all-targets --offline`: clean, no warnings.
- `cargo clippy -j 3 --target x86_64-pc-windows-msvc -p one-mk-input -p one-mk-proto -p one-mk-ctl --all-targets --offline`: clean.
- The equivalent `aarch64-apple-darwin` command: clean.
- `cargo build -j 3 -p one-mk --offline`: passed, debug build only.
- `git diff --check`: clean. No `cargo fmt`, release build, or real input
  backend was run. Tests needed local socket permissions outside the sandbox.

The following is actual output from separate CLI processes and two running
null-backend daemons, with homes under
`/tmp/one-mk-invite-smoke-pkxa7oh_/{A,B}`. The report abbreviates those paths
as A/B and omits the consumed bearer link and ANSI QR rows. Both processes
use the host's real name, so their different fingerprints distinguish them.
A's daemon used port 56900; B's service port is recorded below. Both daemons
used `run --backend null --no-discovery --port <port> --no-clipboard
--no-notify`, with hotkeys disabled in their temporary settings. All
processes were stopped. No real user configuration was accessed.

```text
Active invitation control hook: link + expiry matched through daemon.

Second invitation process: refused by single-instance lock.

$ one-mk --home B join <one-time-link> --service-port 52263
WARN /tmp/one-mk-invite-smoke-pkxa7oh_/B was accessible to others (mode 775); fixing
Authenticated samer-Victus (6d58f87e519fdcdd140724725eaeef20c7e6db96d2c8019104b6dc02c9c4991e). Waiting for approval there...
Trusted samer-Victus (6d58f87e519fdcdd140724725eaeef20c7e6db96d2c8019104b6dc02c9c4991e). Start `one-mk run` on both computers.

$ one-mk --home A invite --no-confirm --expires 2m --listen 127.0.0.1:0 --no-discovery --service-port 56900
Invitation from samer-Victus (6d58f87e519fdcdd140724725eaeef20c7e6db96d2c8019104b6dc02c9c4991e)
Expires at Unix time 1793235995. One use; closing this process cancels it.
Treat this link like a password: whoever uses it first can request trust.
WARNING: --no-confirm: the first valid joiner will be trusted automatically.

>>> samer-Victus joined: certificate 1cd2618fa5adcc604aa96ff7e18614c10782cbc58190edaeb43c901045bb9985 <<<
Invitation consumed. This device is NOT trusted yet.
Trusted samer-Victus (1cd2618fa5adcc604aa96ff7e18614c10782cbc58190edaeb43c901045bb9985).

Consumed invitation control hook: none.

$ one-mk --home A devices
NAME                     ID         SIDE
samer-Victus             1cd2-618f  left

$ one-mk --home B devices
NAME                     ID         SIDE
samer-Victus             6d58-f87e  right

$ one-mk --home A switch next
The mouse and keyboard are going to samer-Victus.

$ one-mk --home A status
one-mk is running on samer-Victus (id 6d58-f87e).
The mouse and keyboard are controlling samer-Victus right now.
  samer-Victus             Linux      on your left   online

$ one-mk --home A switch here
The mouse and keyboard are back on this computer (left samer-Victus).

Default approval + stdin join: no trust before y; refusal saved neither device and removed active link.

Explicit --confirm + y: both devices saved.

Expiry while approval pending: both commands failed; neither trust store was created.

1-second expiry: process exited without stdin; no lingering listener.

All smoke checks passed; all child processes stopped in cleanup.
```

Unverified: physical camera scanning of the terminal QR, mDNS/address
fallback on a real multi-computer LAN, and Windows/macOS runtime behavior.
The QR encoder was checked for its permissive license and dependency set;
no independent cryptographic audit or manual scan is claimed. The intentional
security trade-offs are bearer-link theft/racing, fail-closed denial of
service after five wrong proofs, memory-only invitations, and possible
one-sided trust after a crash around final confirmation, as explained above.
