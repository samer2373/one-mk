# Pairing invitations (3H)

## Use and approval

On A, run `one-mk invite --expires 10m`. Privately transfer its one-use
`onemk://pair?...` link or terminal QR to B. On B, run `one-mk join` and
paste the link. There is no automatic clipboard access or URL handler.
The positional `one-mk join '<link>'` remains supported, but warns that the
credential is exposed in process arguments (including `/proc/.../cmdline`
where readable) and possibly shell history. Prefer stdin and a private,
authenticated transfer channel.

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
interrupted exchange requires a fresh invitation.

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
is shared with one optional busy retry; each dial is at most two seconds.
mDNS gets at most two seconds of the remaining budget. A busy refusal is
retried once after 150–405 ms random delay. None of this promises availability.

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
entries. At capacity, unknown sources are refused. Node retains 16 concurrent
TLS handshakes / 4 per IP and a three-second handshake timeout. Invitation
proofs retain 16 pending attempts / 4 per certificate and a ten-second
exchange timeout. Unknown-ID refusals disclose no name, nonce or proof
validity. Late joins get explicit protocol/QUIC application-close refusals,
including during approval, rather than a ten-second stream timeout.

Both monotonic and wall clocks bound availability. A 250 ms check detects
forward wall jumps while idle; A's approval reader also checks wall expiry.
Either clock expiring ends availability, wipes the retained link and stops
mDNS advertising, with a bounded wait for the daemon's goodbye cleanup.
A valid proof likewise clears the link and withdraws discovery before the
host prompt. B initially checks expiry, but **only A decides invitation
expiry for the final decision**, just before its approval byte. Neither
side rechecks invitation expiry after mutual approval, avoiding one-sided
trust caused solely by crossing expiry then. Independent human/transport
timeouts still apply; an initially skewed B clock can reject a valid link.

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
  erased. This is not a complete process-memory-erasure guarantee.
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
copied-link/same-hostname impostor are covered. The impostor test uses fresh
state to calculate the intended B's hypothetical code; an already-used
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
cargo test -j 3 --workspace
cargo clippy -j 3 --workspace --all-targets -- -D warnings
cargo build -j 3 -p one-mk
cargo build -j 3 -p one-mk-net --example invite_probe
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

`cargo test -j 3 --workspace`: **414 passed, 3 existing ignored, 0 failed**.
Workspace clippy and the following target checks pass with `-- -D warnings`:

```sh
cargo clippy -j 3 --workspace --all-targets -- -D warnings
cargo clippy -j 3 --target x86_64-pc-windows-msvc -p one-mk-net -p one-mk-input -p one-mk-proto -p one-mk-ctl --all-targets -- -D warnings
cargo clippy -j 3 --target aarch64-apple-darwin -p one-mk-net -p one-mk-input -p one-mk-proto -p one-mk-ctl --all-targets -- -D warnings
```

On this Linux host, ring's cross-target C compilation used temporary Clang
21/LLVM tools and genuine Zig 0.13.0 libc headers, not an installed platform
SDK. `CC_<target>`/`AR_<target>` selected those tools; `CFLAGS_<target>` used
`-nostdlibinc -isystem <headers>/any-macos-any` or `any-windows-any`.
The Windows C flags additionally used `-fgnuc-version=4.2.1` so MinGW headers
select GNU-style declaration attributes accepted by Clang's MSVC target.
The Rust targets remained exactly those shown above. No target cfg override,
Rust warning suppression, dependency patch or fabricated header was used.
The net crate's existing Windows-only unnecessary mutable binding was fixed
without changing directory permissions or behavior.

Captured real-process output (invitation bearer links deliberately omitted):

```text
Same-host unicast endpoint: 192.168.1.18 (loopback link literals intentionally rejected)
Public-only attacker: 1000 wrong-ID + 1000 wrong-proof/wrong-ID attempts; 256 explicitly refused, 1744 source-rate-limited; no secret supplied.
Late join: explicit 'invitation already used or expired'; active UI link cleared.
B: Your code: 057186 (the other computer will ask you to compare it)
A: Does the other computer show 057186? Trust it with your keyboard, mouse and shared clipboard? [y/N] -> y
B: Trust samer-Victus (11b2 5e58 834b e49e b62d ebc0 76d2 2f96 e38f 369b 2113 2def 6baf 18bf 87e9 d864)? [y/N] -> y
A devices:
NAME                     ID         SIDE    ADDRESS
samer-Victus             0ff7-4311  left    192.168.1.18:24900
B devices:
NAME                     ID         SIDE    ADDRESS
samer-Victus             11b2-5e58  right   192.168.1.18:24900
Scripted --yes / --no-confirm: both warnings printed; both devices saved.
Joiner refusal: neither trust store created.
Expiry during approval: both commands fail; neither trust store created.
All smoke checks passed; all started processes exited or are stopped in cleanup.
```
