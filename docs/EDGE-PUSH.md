# Edges where no computer sits: pointer wrap and layout learning (5D)

Two features use the edges of this computer's desktop that lead to no
other computer. Without them such an edge is simply not armed, and the
pointer stops there like at the edge of any screen.

- **Pointer wrap** (`[edge] wrap = off | on | all`, `one-mk edge --wrap`,
  default `off`). Pushing the pointer out through such an edge brings it
  in at the opposite edge, like "wrap mouse" in Mouse Without Borders. `on`
  wraps the left and right edges; `all` also the top and bottom.
- **Layout learning** (`[edge] learn = off | suggest | auto`, `one-mk edge
  --learn`, default `suggest`). Pushing the pointer twice within 2 seconds
  against such an edge, while a computer whose place is only a guess is
  online, means "it is over there". `suggest` says so (a desktop
  notification, a line in `one-mk status`, a card on the window's Layout
  tab) and moves nothing; `auto` places it and says how to undo that.

Code: the geometry is pure, in `crates/proto/src/wrap.rs`; the push
counting, cooldown and the choice of computer are pure, in
`crates/one-mk/src/learn.rs`; the daemon side is `crates/one-mk/src/app/push.rs`,
which `app.rs` calls in five places (arming, a capture, a crossing, a
connection, a request). No backend changed.

## One set of armed edges

The daemon already sends the backend the edges that lead to peers
(`CaptureCmd::SetEdges`). The edges where no computer sits are sent in the
same list, after the peers' (so a backend that uses "the first edge of a
side" for `one-mk switch` still finds the peer's). The daemon keeps which
is which; a backend needs nothing new. When a capture starts
(`Event::Activated`), the nearest armed edge says what it is; **an edge
leading to a peer wins whenever it is as near** as a free one, so a free
edge can never take a crossing meant for a computer.

The free edges are the pieces of the desktop's outline (every monitor edge
with no monitor of ours beyond it, `Desktop::exposed_edges`, so also those
set back from the bounding box) minus:

- the edges armed for connected peers (with an offset, where its screens
  touch ours; without, the whole outer side), and
- the whole outer side of every paired computer that is offline or whose
  monitors aren't known yet: it still sits there.

| Edge kind | Armed when | What a capture there does |
|---|---|---|
| peer | as before | crosses (edge rules apply) |
| wrap | `wrap` covers that side | the edge rules decide; crossing releases the capture at the opposite point |
| probe | `learn` is not `off`, the side doesn't wrap, and an online computer's place is a guess | released at once where it is (a wall), and the push counted |

Nothing extra is armed while the pointer is locked to this computer
(`lock-pointer on`) or while a peer controls this computer. A capture that
was already on its way when that changed is handed straight back.

## Wrap

`wrap_point` keeps the pointer on the same row (left and right edges) or
column (top and bottom): out through the right of the rightmost monitor at
height y, in at the left edge of the leftmost monitor on that row, 2 px
inside (the same inset as coming home). Rows are measured over the
monitors themselves, not the bounding box, so on an L-shaped or uneven
desktop the pointer comes in at the outline where that row begins. Where a
row has a gap (a U of three monitors), out through the inner edge of the
gap is in at the far side of it. Property tests on 3000 random
multi-monitor desktops check that the result is always on a monitor, right
inside the outline, and that wrapping back returns to within the inset.

The release uses the existing hand-back path (`CaptureCmd::Release { pos }`),
which every backend honours by moving the pointer there (X11 `XWarpPointer`,
the portal's `Release` with a cursor position, wlroots absolute motion of
its virtual pointer; the Windows and macOS hooks put the cursor there as
they do when the pointer comes home).

## Learning

- A **push** is a run of captures on one side less than 350 ms apart
  (holding the pointer against an edge makes the backend report it again
  and again). **Two pushes** on the same side whose starts are at most 2 s
  apart conclude; a push on another side starts over.
- The **candidate** is, among the online computers whose place is a guess
  and is not that side already, the one that connected last (with one
  online, that one).
- **A guess** (`Device::learnable` in `crates/net/src/trust.rs`): no offset,
  and `placed_by` is `default` with `layout_version` 0 (as pairing leaves
  it) or `learned`. `devices.toml` gains an additive `placed_by`
  (`user`, `learned`, `confirmed`; left out while default; a value this
  build doesn't know reads as default). `one-mk layout`, the window, a
  profile and a layout received from the other computer make it `user`;
  learning makes it `learned`. **Crossing** to a guessed or learned place
  makes it `confirmed`: it was right, and learning leaves it be. Files from
  before `placed_by` read as the user's choice once a layout was chosen.
- **Cooldown:** at most one suggestion or placement per computer every 10
  minutes. A suggestion shows for 5 minutes, or until the computer is
  placed, or until it is dismissed (`one-mk edge --dismiss`, the window's
  "No"). Nothing ticks for any of this: ages are compared when someone
  looks.
- **`auto`** places through the same path as `one-mk layout`
  (`manage::learn_layout`: `devices.toml` with a new layout version, then
  applied, then the mirror image sent to the other computer, which adopts
  it). It acts only on the **left and right**: above and below are where
  the pointer goes for menus, title bars and the taskbar, so a push there
  is only suggested. The notification says how to undo:
  `one-mk layout <name> <previous side>`.

## Interactions

| Situation | Wrap | Learning probe |
|---|---|---|
| Dead corners, dwell, modifier, fullscreen guard (`one-mk edge`) | apply, exactly as for crossing (the same `Approach`) | do not apply: a probe never moves the pointer |
| `lock-pointer on` | not armed | not armed |
| A peer controls this computer | not armed (re-armed when it lets go) | not armed |
| Screen locked | handed back | handed back, not counted |
| Visiting another computer (also gaming mode's relative motion) | cannot fire: the pointer is captured; edges stay armed so nothing changes at every crossing | the same |
| Paired computer offline | its side never wraps | its side is not probed |
| Every online computer placed by the user or confirmed | as set | no probes armed at all |
| Input typed or clicked during the capture | typed here, as at any wall: a key as a tap, a click as a whole click, so nothing stays held | the same |

## Cost

No timer, task or thread. Arming is one more `SetEdges` when the set
changes (a setting, a computer connecting, control starting or ending, a
first crossing). Measured on X11 (Xvfb) with a peer connected, 30 s idle:
1 CPU tick (10 ms) and the same wakeups with `wrap all` and `learn auto` as
with both off; resident memory unchanged (33.7 MB, debug build).

## Backends

| Backend | Status |
|---|---|
| X11 (XFixes barriers) | Tested end to end on a private Xvfb with the real daemon: wrap on every side, a peer's edge still crossing, lock, the probes, suggest and auto with a peer running 0.1.12, `kill -9` mid-visit (no grab left), restart. |
| X11 without barriers (input-only edge windows, old servers) | Compile- and unit-tested only. An edge window covers the edge's last row of pixels, so a probe there takes the click a window under it would get; such servers are rare. |
| Wayland portals (GNOME, KDE) | Untested. The compositor may refuse a barrier on an edge set back from the bounding box (logged as "compositor rejected a pointer barrier"); that piece then neither wraps nor probes. |
| wlroots (Sway, Hyprland) | Untested. |
| Windows, macOS | Compile-checked only. |

`one-mk doctor` (check "Edges with no computer") and `one-mk status` (the
Edges line) say so when wrap or `auto` is on with a backend not tested.

## Limitations

- A probe is a wall that captures: on the portal and X11 a click at the
  very edge pixel in the moment of the capture is replayed as a whole
  click, so a drag cannot start exactly at a probed edge (as at any
  guarded edge). The probes go once every online computer is placed or
  has been crossed to.
- Learning cannot tell a push toward a computer from a flick to the top
  bar; that is why above and below are only suggested, and why a computer
  that was crossed to is never learned again.
- A suggestion lives in the running daemon only (5 minutes); a restart
  forgets it and its cooldown.
- Wrap needs the peer's side known: a computer paired but never connected
  sits on its pairing side (`right` or `left`) until it is placed.

## Manual test on real hardware

On each desktop to check (GNOME and KDE on Wayland, Sway, Windows, macOS),
with two computers A and B running this build:

1. Pair them fresh (`one-mk pair` or invite/join); do not place B. On A,
   `one-mk status` shows B on its pairing side.
2. On A, push the pointer twice within 2 s against an edge where B is not
   (left or right). Expect a notification "Is B on your …?" and the same
   line in `one-mk status`; the pointer stays where it was, and B's place
   is unchanged. Open the window's Layout tab: the card shows the question.
3. Choose **No**: the card and the status line go. Push twice again: no
   new question for 10 minutes.
4. `one-mk edge --learn auto`, restart one-mk on A, push twice against the
   left or right edge where B is not: B moves there (A's status), B's
   status shows A on the mirror side, and the notification gives an undo
   command that works. Push twice against the top: only a question.
5. Cross to B through its new edge and back. Push twice against another
   free edge: nothing happens any more (the place was confirmed).
6. `one-mk edge --wrap on`. Push out through the left edge (no computer
   there) at different heights: the pointer comes in at the right edge at
   the same height. With two monitors of different heights, push out of the
   taller one below the shorter one: it comes in at the taller one's own
   edge. Type a key or click while pushing: it happens on A, and no key or
   button stays held.
7. `one-mk edge --dwell 300 --corner 20`: the wrap waits 300 ms at the edge
   and never happens within 20 px of a corner. `one-mk lock-pointer on`:
   no wrap. Let B control A: no wrap on A until B lets go.
8. Turn B off: A's edge facing B never wraps while B is offline.
9. `one-mk doctor` shows "Edges with no computer" with an "untested"
   warning on that desktop (remove it in `doctor.rs`'s `edge_push_tested`
   once these steps pass there).
