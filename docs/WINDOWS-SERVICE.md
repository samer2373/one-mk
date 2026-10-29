# Windows helper: pure contracts only

**Status:** Linux-tested policy/codec primitives, not a Windows service or a
working helper. No new code runs on real Windows in this task: the Windows
target is compile/clippy-only. No privileged operations, service registration,
pipe I/O, input injection, desktop attachment or process launching are implemented.
Nothing is wired into the daemon, installer or command-line parser.

`crates/input/src/windows_service_logic.rs` is public on Windows and compiled
into input-crate unit tests on every OS. Its four components are independent
of Win32. No doctor hook is added.

## Interfaces for the later, hand-written adapter

- `SessionSource::snapshot` returns the active session ID, logon/lock and console
  connection facts, and `DesktopKind` (`Unknown`, `Default`, `Winlogon`,
  `Screensaver`). Failure is `SourceFailed`, not permission to assume a default
  desktop. Facts must come from trusted OS observations, not pipe requests.
- `HelperControl::start(StartRequest)` and `stop_and_release(generation)` are
  **declarations only**. Requests contain only the provisioned session, helper
  kind and generation. The adapter must enforce authorization independently;
  a Rust request value is not a security capability. Report start failures as
  `HelperExited`, successful starts as `HelperStarted`, and confirmed stops as
  `HelperExited`, always with the original generation. Do not execute stale
  queued starts. No second helper starts until the first has confirmed exit.
- `SessionMachine::step(monotonic_time, event)` returns at most one decision and
  an optional absolute timer deadline. Feed notifications and arm only that
  one-shot timer: there is no idle polling. Clock rollback permanently disables
  this machine. Unknown/disconnected/logged-off/other-user/screen-saver states
  choose no helper. Default desktop requires unlocked state; Winlogon selects
  the secure-desktop helper only for the provisioned, logged-on console session.
- A changed target immediately requests stop/release, then waits 150 ms of
  target stability before starting. Unexpected exits/start timeouts back off
  1, 2, 4, 8, 16, 30 seconds (cap 30); a running minute resets crash history.
  Startup has a five-second deadline. A helper that never confirms stopping
  blocks replacement rather than creating a restart storm. Duplicate/stale
  helper notifications do not change the current generation.

## Local protocol and authorization

`encode`/`decode` use fixed storage/no decode allocations. A four-byte
little-endian body length precedes a header: version `u16` (currently 1),
message type `u8`, reserved zero `u8`. The **entire frame cap is 192 bytes**;
`body_length` rejects an excessive prefix before the adapter reads/reserves a
body. Decode requires exactly one frame: truncation, suffix bytes, unknown
types/versions/flags, noncanonical booleans and nonfinite coordinates fail.

Types are challenge (1: serial `u64` + 32-byte nonce), client proof (2), server
acceptance (3), release-all (4), local-input notification (5), screen-lock
notification (6), and all existing `Inject` variants (16–23). MACs are 32 bytes;
integer/floating payloads are little endian. `Message::from_event` allows only
`Event::LocalInput` and `Event::ScreenLocked`, not arbitrary backend messages.
There are no generic execute, path, environment, service or credential commands.
Event decoding is not itself authorization to act on a helper notification.

The hand-written transport must provide a local-only, OS-authenticated pipe
with restrictive access control and validated endpoints. It must preserve
message order and bind a connection to the authority's challenge serial.
Framing alone does not authenticate anything. Do not expose these frames over
an unauthenticated transport or copy an authenticated serial to another pipe.

`Authenticator` consumes a random 32-byte per-boot credential supplied by the
adapter; pure code does not obtain entropy. Each challenge also needs a fresh
OS-CSPRNG nonce. It enforces a five-second challenge expiry, 24-hour credential
expiry, three failed proof attempts, one-use challenges and strictly increasing
serials. Keep its budget/serial state across reconnects; do not recreate it to
retry or share it between concurrent pipes. Its policy context pins one user
network identity, paired-device identity, session and helper kind. Reprovision
only through the trusted authority when that context changes, retaining the
boot-wide retry/replay limits. Secrets must not appear in logs or arguments.

Proof and acceptance are domain-separated HMAC-SHA256 using existing `ring`.
The 160-byte, zero-padded transcript contains the one-byte domain length,
domain, protocol version, user identity, paired identity, session, helper kind,
serial and nonce, in that order. Verification uses `ring::hmac::verify`
(constant-time MAC comparison). `ClientCredential` constructs the proof and
verifies the distinct acceptance MAC; the caller must require acceptance for
its outstanding challenge before sending commands. It is not a network key.

`Authenticator::input` accepts only input/release messages on the authenticated
serial, attributed to the pinned paired device. Bad authentication/state,
expiry, clock rollback, wrong peer or malformed input fail closed. A reconnect
invalidates the old serial. The daemon must first authenticate the remote peer
with existing mutual TLS/pairing policy: a caller-supplied fingerprint is not
proof of a remote identity. Arbitrary helpers must not request new peers.

On every pipe failure, auth error/expiry, switch, stop or disconnect, the
adapter must release all held keys/buttons and discard queued input before
allowing further input. `ReleaseAll`/`StopAndRelease` express that contract;
pure logic cannot release OS input. The adapter must also enforce bounded
queues/read deadlines and handle a stalled or crashed process without polling.

## Fixed launch contract and security boundary

`helper_command_line` permits only
`C:\Program Files\one-mk\one-mk-helper.exe` (ASCII-case-insensitive exact
match), with fixed fields for session, kind and generation. This future binary
and these flags **do not exist yet**. No token is put on the command line.
`command_line` implements Windows CRT argument quoting, rejects NUL and caps
the result at 32,766 UTF-16 units plus the terminating NUL. It is not shell
escaping; never pass it to a shell. The adapter must specify the separately
validated executable rather than asking Windows to infer it from argv[0].

The path validator rejects relative paths/components, UNC/device paths, 8.3
aliases, alternate data streams, prefix lookalikes and trailing dots/spaces.
It verifies **lexical canonical spelling only**. Trusted installation, actual
Program Files resolution, filesystem identity/reparse-point checks, executable
integrity and write-protected installation/IPC policy remain manual work.
Alternate installation drives are deliberately unsupported by this allowlist.

Only paired-device input is admitted by the pure gate. It cannot prove OS
caller identity or make an untrusted daemon privileged. The hand-written part
must ensure a standard user cannot configure an executable, replace a helper,
change protected policy, choose another user's session, mint grants or cause
arbitrary privileged execution. Keep **one network identity per user** in the
user daemon; the helper receives no TLS private key, pairing authority or
network listener. Access-control enforcement and secure credential provisioning
must be hand-written and independently reviewed, not inferred from these tests.

## Phase split

1. **User-session lock screen and UAC:** existing per-user daemon retains QUIC,
   mutual TLS, pairing and its single network identity. A manually implemented,
   reviewed authority/transport/helper adapter supplies the contracts above.
   These pure decisions do not establish that Windows permits injection there.
2. **Sign-in screen with no user daemon:** not implemented or selected by the
   current state machine. Requires a separately reviewed identity/admission and
   lifecycle design without borrowing a user's private key. No automatic
   fallback to a machine-wide network identity.

Before either phase ships: hand-write/review all privileged runtime pieces,
installer/service integration and policy; then test real Windows logon/logoff,
lock/unlock, UAC transitions, console/user switching, crash recovery and held-key
cleanup. Linux unit tests and target clippy cannot validate any of that.

## Validation (2026-10-29)

27 new pure-logic tests pass on Linux. `cargo test -j 2 --workspace` passes
661 tests with 6 existing ignored tests and no failures. Strict clippy
(`-- -D warnings`) passes for the workspace/all targets and for
`one-mk-input`, `one-mk-proto`, `one-mk-ctl` with `--all-targets` on both
`x86_64-pc-windows-msvc` and `aarch64-apple-darwin`, always with `-j 2`.
Cross-target C compilation uses the existing temporary Clang/LLVM tools and
genuine Zig libc headers described in [PAIRING-INVITE.md](PAIRING-INVITE.md);
there is no Windows/macOS runtime validation or installed platform SDK.
