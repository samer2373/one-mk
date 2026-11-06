# Design council: the Claude branch (`council/claude`)

What this branch changes in the window (`crates/app`), what is still weak,
and the numbers measured. Screenshots: `/home/samer/omk-council/claude/round2/`
(the same file names as `scripts/ui-shots.sh`, plus the evidence shots below).

## What changed

**Tokens and controls**
- `GROUP`, `TRACK`, `THUMB` and `BUTTON` are opaque. Segmented tracks, fields
  and switches that are off have a 3:1 edge (`TRACK_LINE`, `FIELD_EDGE`, `SWITCH_EDGE`).
- Every control's hit area is at least 24 × 24 pt about its centre
  (`widgets::hit_rect`, `MIN_TARGET`).
- Every control family has a pressed look, separate from hover, selection
  and focus (`widgets::pressed`). Fills go deeper, Volt deepens toward its
  ink, a segment cell shows a dark well and the slider knob dims.
- The primary capsule is flat, solid Volt, with no gradient, rim or glow.
- Shortcut hints use `#7D8692`: 5.2:1 on the sidebar, 4.6:1 on its chosen row
  and 4.8:1 on a hovered row.
- Unit tests check text at 4.5:1 and control edges at 3:1. They measure the
  colour after compositing, never a premultiplied token's channels:
  `style::tests::text_meets_four_and_a_half_to_one_where_it_is_drawn` and
  `control_edges_meet_three_to_one`. Other tests cover hit areas and pressed fills.

**Desk**
- The camera fits every corner of every monitor, stand and base, plus the
  pairing slot, 12 pt inside the desk on every side. It moves sideways only as
  far as that needs. Tested: `scene::tests::every_part_fits_inside_the_desk_area`.
- Faces are drawn per computer, the farthest computer first, so no seam cuts
  through a nearer monitor while dragging.
- A band at the top of the desk holds the banner, the toast and, in a small
  window, the transfer. It eases open and the desk is framed below it.
  Transfers and the drag hint along the bottom have room kept free for them too.
- While another computer controls this one, the banner has a named
  **Take back** button.
- An error toast offers **Open Health**.
- When the desk is under 128 pt tall, the computers show as named chips
  (pointer, status, where each sits) instead of tiny monitors.
- Colour meanings:
  - **Volt** is used for the crossing ribbon, changed and chosen keys, the
    switch that types on every computer, and the primary action.
  - **Notices** get a neutral dot; it is Volt only while broadcasting.
  - **Pending** shows as a clock ring in ink.
  - **The guide's waiting dot** is ink.
- The idle timer is a clock icon with the word **Timer**.
- In the peer card, measuring again sits beside the number. A computer with
  no readings shows one quiet line instead of empty axes.
- The "This computer" card explains pairing when nothing is paired.
- When oneMK is off, its card holds **Start oneMK** (Volt). The sidebar's
  matching button turns quiet so only one Volt button shows.

**Pages**
- **Layout in a narrow window:** the cards stack, each as tall as what it holds
  and full width, so no choice is shortened. The column scrolls. The profiles
  list ends on a whole row.
- **Clipboard & files:**
  - It is one scrolling column of opaque groups, each as tall as its contents.
  - "Others can get this clipboard" is shown in full.
  - History items are two-line rows; no row is cut by an edge.
- **Health:**
  - The page is split into Needs you, Pending and Working.
  - A permission dialog still to answer is listed under Pending; it never
    gets a tick. On macOS it offers Allow sharing.
- **Settings:**
  - Each shortcut row has its name on the left and its keys in a recessed
    mono recorder; there are no repeated "Change" buttons.
  - Ctrl and Cmd always shows the full choices, on a row of their own when
    the card is narrow.
- **Keys & mouse:**
  - Mouse labels sit on near-solid plates, drawn after the leader lines.
  - Quick setups are solid capsules with a Volt tick.
  - Changed legends are Volt.
- **Help explainers** show their picture in its own well. It loops at
  30 fps instead of 60, and stops under reduced motion.

## Measured

Measured on GNOME Wayland at 1.25× (`/home/samer/omk-council/claude/perf.txt`).
The CPU figures are the process's user and system time from `/proc`.

| what | result |
|---|---|
| Release build (no LTO), window visible, at rest, 60 s | 0.10 % of one core |
| Debug build, desk with three computers, visible, 60 s | 0.73 % |
| Debug build, minimised, 30 s | 0.03 % |
| Debug, help explainer open (picture looping): before → after | 27.1 % → 14.3 % |
| Moving frames (`ONE_MK_APP_BENCH=300`, debug): before → after | p95 13.12 → 13.24 ms; CPU/frame 7.13 → 6.17 ms |
| Frames asked for at rest, 5–8 s, Desk / Settings / Keys / Files | 0 (before and after) |

## Keyboard

`ONE_MK_APP_TABWALK=n` (a debug aid) sends real Tab key events and prints the
order. The full lists are in `round2/tab-order-desk.txt` and
`round2/tab-order-advanced.txt`. Shots with the focus ring:
`round2/kb-desk-tab*.png` and `round2/kb-settings-tab*.png`.

The Desk order is (as recorded):
1. The computers on the desk.
2. Search.
3. The pages.
4. This computer and the paired computers in the sidebar.
5. The sidebar's action, its status line (it opens Health) and Stop.
6. The Computers/Arrange switch.
7. The toolbar, from right to left: Lock every screen, Timer, then Type on every computer.
8. The card: Remove, the position choices, Measure again, Switch to it, Send and Get.
9. This computer's switches and their explainers.

The toolbar's right-to-left order, and the desk coming before the sidebar,
follow the order things are drawn in. They are left as they are (not a
design change).

On Settings, the number fields show as "(nothing named)" in the debug list.
They do have accessible names, but the debug target list doesn't record them.

Not reachable by Tab: the latency graph (it is read out as text), the scene's
drag gestures (the Left/Right/Above/Below control and the arrow keys on a
focused computer do the same), and the help pictures.

## Evidence shots

- **Scale:** `s100-*`, `s125-*`, `s150-*`, `s175-*`, `s200-*`, each for Desk,
  Settings and Keys. The desktop here is 1.25×, multiplied by the app's zoom.
- **Reduced transparency:** `rt-*`.
- **Reduced motion:** `rm-*`.
- **Minimum size:** `min-820-*` and `min-760-layout-scrolled`.
- **Scrolled Files:** `files-scrolled`.

## Still weak

- There is no 200 % text-size setting in the app, so no capture of one. Zoom
  scales everything instead.
- No screen reader was run. The roles and names come from the code and the
  debug target list, not from hearing Orca, NVDA or VoiceOver.
- macOS and Windows are cross-checked only, not seen.
- Desk chips replace the 3D desk in very small windows. Dragging isn't
  available there; the position control in the card is.
- The keyboard picture on Keys can sit partly scrolled under the page header
  when an editor is revealed.
