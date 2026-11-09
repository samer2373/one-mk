# Idle wake-ups: measured, not inferred from CPU ticks

## Result and scope

Measured on Linux in October 2026 on a development build (debug, unoptimized). With no
discovery and no peers, both the `null` backend daemon and the X11 backend daemon had **zero
voluntary or involuntary context switches on every thread over 60 seconds**. An idle connected peer,
on either backend, produced approximately 4.3-4.7 voluntary context switches per second on the
daemon's main thread (its QUIC keep-alive and acknowledgements), not on the X11 thread. A report of
"four wake-ups a second on X11" is therefore the cost of a connected peer, not an X11 problem.

The one remaining periodic source is the mDNS library's own network-interface check (every 5 seconds,
with discovery on). Switching it off without an equivalent network-change notification would break
discovery after DHCP and Wi-Fi changes, so it stays; `--no-discovery` removes it. The literal
"zero work when idle" statement is true for the measured disconnected, no-discovery service, not for
the default discovery-enabled service or a live peer connection.

## Method and safety

- Both passes use the same locally built `target/debug/one-mk` (`cargo build -p one-mk`), dev
  profile with debug information; no release or LTO build was used.
- Each daemon has a separate throwaway identity, trust store, HOME, runtime
  directory (0700), and explicit `--home`; no process uses the real one-mk
  configuration. Child environments are constructed from scratch: PATH, HOME,
  XDG_RUNTIME_DIR, ONE_MK_HOME and private DISPLAY only. No Wayland or session
  D-Bus environment is inherited. No tray or application window is launched.
- Xvfb chooses an unused display with `-displayfd`, disables TCP listening, and
  is terminated/reaped after its children. Null scenarios also use that private
  display when available. Only processes started by the helper are stopped.
- Each backend is measured with no paired peer, one paired peer at a closed
  loopback UDP port, and one real connected peer. Clipboard sharing is enabled;
  its worker is lazy until a clipboard request. The peer daemons do not enable
  discovery. Each measured daemon is tested with discovery disabled and enabled.
- Allow 20 seconds to settle, then take before/after snapshots of **each TID's**
  `/proc/PID/task/TID/{comm,status,stat}` over 60 seconds. Report voluntary
  context-switch deltas divided by the monotonic elapsed seconds; involuntary
  context switches and CPU ticks are recorded separately, not called wake-ups.
  These counters measure voluntary blocking/scheduling transitions, not an exact
  count of hardware interrupts or all possible scheduler wake events.
- Scenarios run as a cohort, not serially. The first pass had six discovery
  advertisers; the repeat discovery pass had three. mDNS traffic from the LAN
  and the cohort is deliberately not filtered. Consequently mDNS before/after
  differences are **not** a performance comparison. No Internet service is
  contacted; discovery does use LAN multicast.
- `strace -p` was refused by ptrace policy even outside the filesystem sandbox.
  Launching the daemon *as a child of strace* worked. Separate 85-second traces
  covered no-peer, offline-peer, connected-peer and mDNS cases, with 20 seconds
  excluded as settling time. Traced runs are not used for the counter budgets:
  ptrace itself perturbs scheduling. The requested `%desc,%process,futex,
  epoll_wait,poll,ppoll,select,clock_nanosleep,nanosleep` filter showed blocking
  waits and timer expiration; it is not a packet capture.

### Per-thread results

Each entry is **first pass → repeat**, voluntary context switches/second.
Counts in parentheses are raw switches over 60 seconds. All rows refer to
the primary daemon, not the additional daemon used as its connected peer.
Linux truncates `comm` to 15 visible bytes (`one-mk-clipboar` below).

| Backend | Discovery | Peer state | `one-mk`: main/Tokio/network/control | `mDNS_daemon`: multicast and interface checks | `one-mk-mdns`: resolved-event forwarder | `one-mk-clipboar`: clipboard requests | `one-mk-x11`: input | `one-mk-drop`: XDND |
|---|---|---|---|---|---|---|---|---|
| null | off | none | 0 → 0 (0 → 0) | absent | absent | 0 → 0 | absent | absent |
| null | off | offline | 0.717 → 0.717 (43 → 43) | absent | absent | 0 → 0 | absent | absent |
| null | off | connected | 4.417 → 4.617 (265 → 277) | absent | absent | 0 → 0 | absent | absent |
| X11 | off | none | 0 → 0 (0 → 0) | absent | absent | 0 → 0 | 0 → 0 | 0 → 0 |
| X11 | off | offline | 0.700 → 0.683 (42 → 41) | absent | absent | 0 → 0 | 0 → 0 | 0 → 0 |
| X11 | off | connected | 4.400 → 4.600 (264 → 276) | absent | absent | 0 → 0 | 0 → 0 | 0 → 0 |
| null | on | none | 0.017 → 0.017 (1 → 1) | 0.917 → 0.500 (55 → 30) | 0.050 → 0.050 (3 → 3) | 0 → 0 | absent | absent |
| null | on | offline | 0.733 → 0.733 (44 → 44) | 0.933 → 0.500 (56 → 30) | 0.050 → 0.050 (3 → 3) | 0 → 0 | absent | absent |
| null | on | connected | 4.383 → 4.283 (263 → 257) | 0.900 → 0.483 (54 → 29) | 0.050 → 0.050 (3 → 3) | 0 → 0 | absent | absent |
| X11 | on | none | 0.017 → 0.017 (1 → 1) | 0.900 → 0.483 (54 → 29) | 0.050 → 0.050 (3 → 3) | 0 → 0 | 0 → 0 | 0 → 0 |
| X11 | on | offline | 0.733 → 0.733 (44 → 44) | 0.867 → 0.483 (52 → 29) | 0.050 → 0.050 (3 → 3) | 0 → 0 | 0 → 0 | 0 → 0 |
| X11 | on | connected | 4.667 → 4.267 (280 → 256) | 0.900 → 0.433 (54 → 26) | 0.050 → 0.050 (3 → 3) | 0 → 0 | 0 → 0 | 0 → 0 |

The measured main thread is the daemon's current-thread Tokio runtime, **not**
a separate input thread. A context-switch total for the PID must not be
attributed to X11 merely because `--backend x11` was selected.

No-discovery/no-peer CPU tick deltas were also zero on all threads. Connected
main threads consumed only 4–5 ticks in a minute despite 256–280 voluntary
switches. This demonstrates why a short zero-tick sample cannot establish
zero wake-ups. Other than the explicitly zero rows, involuntary counts varied
from 0 to 5 per TID/window; they are not used to decide whether code polls.

## Source attribution and what remains

| Source / call site | Evidence | Decision / cost and benefit |
|---|---|---|
| X11 input: `crates/input/src/x11.rs`, `X::session`; `crates/input/src/linux_rt.rs`, `Socket::readable` | `one-mk-x11` stayed at 0 in every counter window. After startup its epoll wait remained blocked; no 250 ms timeout. `has_data` uses **zero-timeout poll only after readiness**, not a periodic loop. | Already event driven. Do not replace it with a timeout or change XI2 event handling. |
| XDND: `crates/input/src/dropzone_x11.rs`, `run` | `one-mk-drop` stayed at 0. | Existing timeout is guarded by an actual pending drag deadline. No idle timer. |
| Focus: `crates/input/src/system/linux.rs`, `serve` / `wait` | Not started in these default idle scenarios. Code uses `poll(..., -1)` while watching and no change/debounce is pending. | On-demand probes can have a bounded request wait; the worker has a one-shot 60 s idle retirement, not a heartbeat. Focus-watch idle after an active visit is not established by this matrix. |
| Clipboard: `crates/clipboard/src/lib.rs`, `Clipboard::system`; arboard 3.7.0 `platform/linux/x11.rs`, `serve_requests` | Lazy clipboard worker stayed at 0. Arboard's serving thread blocks in `wait_for_event`, not selection-owner polling. Clipboard reading has a 1 ms retry **only while a requested conversion is pending**. | Keep conversion timeout and selection-request service. This is not evidence about every external clipboard manager or a live desktop. |
| Offline retry: `crates/one-mk/src/app.rs`, `dial_missing`; `crates/net/src/node.rs`, `connect_timeout` / `transport` | Main thread 41–44/min. Trace has bursts of progressively longer epoll deadlines separated by long waits. | Backoff is 2, 4, 8, 16, then 30 s; handshakes have a 3 s caller deadline, 2 ms initial RTT, and QUIC loss/PTO/draining timers. Not an idle housekeeping poll. Removing retry breaks reconnect with discovery off. Changing initial RTT or retry deadlines requires latency/loss tests; unchanged here. |
| Connected QUIC: `crates/net/src/node.rs`, `transport`; Quinn driver / quinn-proto `Timer::KeepAlive`, `Timer::MaxAckDelay`, loss/idle timers | Main thread ~4.3–4.7/s on **both** backends. Trace repeatedly has approximately 1 s waits, socket-readiness returns, and approximately 25–30 ms timer waits. Attribution to keepalive/ACK handling is supported by transport settings and Quinn source; a syscall-only trace does not label an individual Rust future. | 1 s keepalive, 4 s max idle timeout. PING, ACK, delayed-ACK and driver scheduling explain why “1 ping/s” is not “1 context switch/s.” Retain existing four-second dead-peer safety and active-visit behavior. |
| mDNS: `crates/net/src/discovery.rs`, `Discovery::start` / `browse`; mdns-sd 0.21.6 `service_daemon.rs`, `run`, `schedule_ip_check` | `mDNS_daemon` 26–56/min including LAN traffic. Quiet trace segments include an epoll **5000 ms timeout returning 0**, followed by interface enumeration. | **Real remaining periodic poll:** dependency checks interface addresses every 5 s (12/min absent coalescing). Also legitimate browse/probe/retransmit, TTL refresh and inbound multicast work. `--no-discovery` removes the entire source. Turning off `set_ip_check_interval` without replacement misses DHCP/Wi-Fi changes. A safe future fix needs kernel route/address notifications (Linux netlink and equivalent Windows/macOS APIs) wired to a supported rescan API; the dependency currently exposes no direct rescan operation. Not fixed. |
| Discovery forwarder: `Discovery::spawn_browser` | `one-mk-mdns` 3/min in both cohorts; blocking `events.recv()`. | Wakes on resolved announcements, not on its own timer. LAN-dependent; no strict universal budget. |
| Power, shortcuts and portal tasks | No persistent extra thread/wake source in the clean private session. They wait for signals or return unavailable when there is no session bus. | This run does **not** exercise a real desktop portal, logind lock changes, or a live session D-Bus. No polling was found in their idle code paths. |
| Network profiles: `crates/one-mk/src/netwatch.rs` | Disabled by default; not started without automatic profile rules. | NetworkManager signal watcher is event driven. No independent idle tick identified. |
| Control socket, inbox, transfers, settings/trust reload, hooks | No-peer main thread 0. `TRUST_RELOAD_INTERVAL` is a throttle when handling events, not an interval task; drop-job joins are guarded by nonempty jobs. | No idle accept timeout or housekeeping interval to remove. Transfer progress and hook timeouts only run while requested work is pending. |

### Can keepalive be made slower or adaptive?

Not by simply choosing 10–15 seconds: the **four-second QUIC idle timeout**
would expire a healthy idle connection first. Increasing both values would
weaken the documented dead-peer/disconnect target. During an accepted visit,
the application additionally sends 20 ms heartbeats and enforces a 750 ms
owner lease; neither is an idle timer and neither was changed.

A more ambitious design could use a slower transport for idle links and keep
the existing active-visit safety separately, with explicit transition and
loss-of-peer tests. It must distinguish “no visit” from “safe to miss a dead
peer for 15 s,” and retain reconnect and control-stream semantics. That design
is not implemented or claimed validated here. A keepalive also produces
traffic that can refresh some network/NAT state. **These measurements do not
establish that a 1 s keepalive is needed to prevent Wi-Fi power saving**, or
that keeping a radio awake would be desirable; that requires radio/hardware
measurements and is not a justification for silently weakening safety.

## Regression test and reproduction

`crates/one-mk/tests/idle_wakeups.rs` runs the helper as part of `cargo test` on
Linux. Python 3 is required; missing Python prints a skip reason. Missing Xvfb
prints a clear X11-only skip and still tests null scenarios. An installed Xvfb
that fails to start is a **failure**, not a silently passing skip. Use
`-- --nocapture` to see the per-thread measurements and skip messages.

```sh
cargo build -j 2 -p one-mk
python3 scripts/test-idle-wakeups.py target/debug/one-mk \
  --seconds 60 --report idle-direct.json
python3 scripts/test-idle-wakeups.py target/debug/one-mk \
  --seconds 60 --discovery --report idle-mdns.json
cargo test -j 2 -p one-mk --test idle_wakeups -- --nocapture
```

All subprocess isolation is implemented in the helper; never substitute the
live DISPLAY, HOME, or session bus to make the test pass. Setup waits are
bounded, children are killed/reaped on success and exceptions, temporary
files are small, connected status is checked before and after sampling, and thread
creation/disappearance during a measurement is reported as a failure.

Default settle/sample lengths are 20 s each. The no-discovery budgets below
are **per thread per 20-second sample**, scaled linearly for longer windows:

| Thread/scenario | Budget | Rationale |
|---|---|---|
| No peers: every thread | ≤6 voluntary switches (0.3/s) | Observed 0; allows small startup/IPC residuals, rejects even a 1 Hz poll. |
| Offline peer: main | ≤64 (3.2/s) | Retry bursts need headroom for window alignment, scheduler delays and timeout variance; observed 41–44/min over 60 s. A continuous added 4 Hz poll exceeds it. |
| One connected peer: main | ≤128 (6.4/s) | Observed ~4.3–4.7/s; allows normal transport variability but rejects an additional 4 Hz poll. Not a proposed zero-idle guarantee. |
| Any non-main thread, any direct scenario | ≤6 (0.3/s) | In particular prevents hiding an X11 poll underneath the main thread's QUIC budget. |

Involuntary preemption is recorded, never budgeted. Discovery mode reports
counts but does not assert traffic-dependent rates. The connected budget is
for exactly one peer and no visit, not arbitrary peer counts or active traffic.
Debuggers, profilers and deliberate input do not belong in the assertion run.

## Limits of these measurements

- The counters measure voluntary scheduling transitions, not hardware interrupts. They were taken on
  one Linux machine with a debug build; a release build, other desktops, Windows and macOS were not
  measured.
- A real desktop portal, logind lock changes and a live session bus were not part of the matrix
  (no polling was found in their idle code, which waits for signals).
- Two real X11 daemons on separate private X servers switched and returned control, transferred a file
  with byte-for-byte verification and moved text with `clip-send`; that is a smoke test, not a
  replacement for hardware loss, latency, hotkey or portal tests.
- Whether a 1 second keep-alive helps Wi-Fi radios stay awake was not measured; that needs radio
  measurements.
