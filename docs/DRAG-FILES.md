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

Implementation and verification status is recorded below as work completes;
feasibility is not evidence that a native adapter has been implemented.
