# Edge file drops: feasibility and contract

Study: 2026-10-29. This is a **copy to the receiver's configured Downloads
inbox**, not a move, remote pointer drag, or injection into a remote file
manager. The source application's drag completes on a local drop target;
network delivery happens afterwards and may fail. Never advertise move.

## Desktop feasibility

- **X11:** XDND sources own `XdndSelection`; its owner alone does not prove
  a file drag (ask `TARGETS` for `text/uri-list`). A legitimate target receives
  `XdndEnter`, `XdndPosition`, `XdndDrop`, requests the URI selection with the
  drop timestamp, and replies `XdndFinished`. An override-redirect **InputOutput**
  window is needed: InputOnly windows cannot have a visible background.
  Read selection data only after a drop; never read the real clipboard.
  XI2 raw button events can arm observation of motion during a drag even when
  the file manager has a pointer grab. Capture must not steal that grab.
- **GNOME/KDE Wayland:** the compositor sends a data offer only to the client
  under the drag. InputCapture/RemoteDesktop portals do not grant arbitrary
  access to another client's drag, its file list, or global button state.
  A normal focused application can receive a drop on its own visible window,
  but cannot reliably implement this global, dynamically armed edge zone.
  No supported edge-drop implementation is claimed. XWayland does not make
  native Wayland drags globally visible.
- **wlroots Wayland:** layer-shell can place a surface at an edge and that
  surface may receive compositor-mediated DND when supported. Neither
  data-control nor virtual-input protocols provide global drag-data snooping
  or the required global held-button observation. A permanent user-visible
  drop shelf or compositor extension is a different design, not implemented.
- **Windows:** `OleInitialize` on a message-pumping STA, an edge HWND and
  `RegisterDragDrop(IDropTarget)` legitimately receive `IDataObject` on drop.
  Accept `CF_HDROP` filesystem paths, copy only. A low-level input hook cannot
  obtain another process's data object. Shell virtual files (mail attachments,
  cloud placeholders requiring materialization) need an additional explicit
  data-object implementation and are not supported. Integrity boundaries,
  secure desktop, and session-0 services cannot be bypassed.
- **macOS:** an AppKit window/view registered for file URL pasteboard types
  implements `NSDraggingDestination`; read its dragging pasteboard only in
  `performDragOperation:`. Global input observation requires Accessibility /
  Input Monitoring as appropriate; AppKit windows and their run loop belong
  on the main thread. Finder promises, sandbox security-scoped URLs, and
  cross-user/lock-screen drags require additional handling; do not fake them
  by reading the general pasteboard.

## Safety and lifecycle

Only user-dropped local absolute filesystem paths are accepted. Reject remote
URI hosts, non-file schemes, malformed percent escapes, query/fragment parts,
NULs, symlinks (including ancestor components), special files, unreadable files,
and paths the existing receiver rules cannot represent. Bound URI data before
allocation, recursion depth, file count (10,000) and total bytes (1 TiB).
Deduplicate inputs. Revalidate at transfer collection; filesystem mutation by
the same user remains a race, not a sandbox boundary against that user.

Arming is event-driven: primary button held in a connected peer-facing span
shows a thin zone; leaving/canceling/disconnecting hides it. Repeated motion
in the same span produces no repeated map/unmap. A drop is single-use. A
bounded release grace allows the source's asynchronous `XdndDrop` to arrive
after the raw button-up. No periodic timer, motion subscription, or mapped
zone is needed while idle. Existing resumable transfer and progress tracking
are used rather than a second network protocol. Completion must say
`Sent N files to <computer>` only after all files are acknowledged.

## Primary specifications

- XDND: https://www.freedesktop.org/wiki/Specifications/XDND/
- Wayland data-device: https://wayland.freedesktop.org/docs/html/apa.html#protocol-spec-wl_data_device
- Portal interfaces: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.InputCapture.html
- Windows: https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-registerdragdrop
- AppKit: https://developer.apple.com/documentation/appkit/nsdraggingdestination

## Implementation and verification (2026-10-29)

The shared, unit-tested state machine debounces repeated samples, hides on
leave/button-up, latches Escape until release, and accepts each drop once.
Connected peer-facing spans are derived from the existing screen layout;
locked, remotely controlled, disconnected, and disabled states clear zones.
Settings are saved through the existing locked `Settings::update` path:

```toml
[drag]
enabled = true
zone_px = 12
```

`one-mk drag --on`, `--off`, and `--zone 1..128` update these settings and ask
the daemon to reload. Defaults enable only supported desktops. `doctor`
explains Wayland unavailability; explicitly enabling the setting cannot make
an unsupported backend work. No window/tray editor has been added.

- **X11 implemented and tested:** a private Xvfb with XTest input and a
  scripted XDND source verified raw-button/motion arming, actual target-window
  mapping, URI offer negotiation, selection conversion after drop, two file
  URLs, copy acknowledgement, and unmapping. This is not evidence of manual
  Nautilus/Dolphin/Thunar interoperability. INCR selection streaming is not
  implemented; oversized, incremental, non-file, and non-copy offers fail
  closed. Drop onto the zone before crossing the pointer-handoff boundary;
  this does not turn an ordinary remote pointer visit into a file drag.
- **Windows implemented, compile-checked only:** a separate OLE STA owns a
  topmost no-activate edge window, `IDropTarget`, and chained input hooks.
  Native Explorer drop, DPI/multi-monitor behavior, permissions, and interplay
  with pointer handoff require real Windows verification. Windows reparse
  points (including junctions) are rejected. Existing Windows notifications
  currently print to the daemon log, not a newly implemented toast service.
- **macOS implemented, compile-checked only:** an AppKit main-thread run loop
  owns an accessory window/view, global/local event monitors and file-URL
  `NSDraggingDestination`. Finder interoperability, permissions, display
  scaling/multiple screens, and pointer handoff require real macOS verification.
  Missing permission/monitor initialization fails gracefully. Neither adapter
  reads the clipboard or promises delivery before the transfer acknowledgements.
- **Wayland:** deliberately no edge adapter, no XWayland fallback, no
  compositor-specific hacks. A normal app window can receive a deliberate
  drop; globally arming this hidden edge zone still needs compositor support.

Safe intake rejects missing/relative/traversal/remote/symlink/special paths,
validates nested trees and readable regular files, collapses duplicate roots,
and bounds depth (64), tree entries (20,000), files (10,000), bytes (1 TiB),
URI payload (1 MiB) and the existing receiver path rules. Empty directories
are not represented by the existing transfer format. The queue and concurrent
sends are bounded (four). Progress uses the existing transfer reporter; only
a fully successful batch prints/notifies `Sent N files to <computer>`.
OS drop acknowledgement means accepted for local processing, not guaranteed
remote delivery; safety/connection/transfer failures are reported separately.

The ignored test `drag::tests::two_null_daemons_receive_a_simulated_edge_drop`
starts **two real binaries** in throwaway homes with `run --backend null
--no-discovery --no-notify --no-clipboard --port <free> --downloads <private>`.
The harness arms the shared state machine, simulates a drop, and invokes the
same safe-intake/send handler used by the daemon; native intake is separately
covered by Xvfb, not simulated inside the null backend. It verifies received
bytes and kills/waits both children on every exit. Run after building:

```sh
cargo build -j 2 -p one-mk --bin one-mk
ONE_MK_TEST_BINARY="$PWD/target/debug/one-mk" cargo test -j 2 -p one-mk \
  drag::tests::two_null_daemons_receive_a_simulated_edge_drop -- --ignored --nocapture
```

Observed output (terminal progress control codes omitted):

```text
Sending 1 file (55 B) to computer-1 (127.0.0.1:49095)
Sent 1 file (55 B) to computer-1 in 0.1 s (603 B/s)
Sent 1 files to computer-1
PASS: two real --backend null daemons; armed edge -> simulated drop -> safe intake -> QUIC -> receiver Downloads; 55 bytes verified
test drag::tests::two_null_daemons_receive_a_simulated_edge_drop ... ok
```

Validation: workspace tests and clippy; Windows/macOS cross-target clippy for
`one-mk-input`, `one-mk-proto`, `one-mk-ctl` with `--all-targets`. Native runtime
verification is **not** implied by a cross-target check. No live input backend,
real clipboard, or real user configuration was used. There are no idle polling
timers in the adapters; idle CPU and footprint have not been benchmarked.
