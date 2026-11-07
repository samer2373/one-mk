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

## Round 3

**Changes**
- **Help explainers:** the picture plays twice when an explainer opens, then
  rests on its last frame. An open explainer now uses no CPU at rest.
- **Health:**
  - The headline names its groups: "3 need attention · 1 awaiting permission".
  - The shortcut line shows once, not twice.
  - Every row that isn't a pass has an action: the window's own fix, Copy
    command, Recheck, or Open guide. On macOS, Open guide opens the
    permission guide. On other desktops it opens the steps for the desktop's
    own dialog, in a popover.
  - A permission still to answer is listed under Pending, never with a tick.
- **Colour meanings:** the newest latency reading, the live sparkline dot and
  the Connected dot are cyan, never Volt.
- **Narrow Layout:** Position, Exact place and Profiles are tabs over one
  full card, so all three are a click away.
- **Desk:**
  - Named chips replace the desk below 128 pt plus 22 pt for each other
    computer. The chips take dropped files.
  - Clipboard & files in a narrow window shows the chips at the top, and the
    page gets the room.
  - Error toasts have **Try again**, which re-runs the move, swap, Take back,
    Start, setting, start-at-login change or folder opening that failed. They
    also keep **Open Health**.
  - "Few readings so far" replaces "Measuring…" next to a live number.
- **Room and cards:**
  - The room is quiet: it has no lights of its own, the screen glow is
    faint, and the desk is a neutral dark.
  - Desk cards and the Layout card are flat groups.
  - The monitors always start below the toolbar row.
  - The band spans the desk, so its right edge lines up with the cards.
- **Type and layout fixes:**
  - The sidebar uses the 12-point step.
  - Files rows are 36 pt with hairlines.
  - The Health column starts under the page title.
  - The Keys close button sits on its card's title line.
  - Changed-key legends sit inside the cap.
  - Mouse labels go to the side their part is on.
  - Monitor names shrink before they are shortened, and a shortened name
    shows in full on hover.
  - An offline computer is described in words.
  - An open profile's Wi-Fi network sits on its own line.
- **Focus:**
  - Tabbing onto any control scrolls it into view.
  - Pressing the chosen segment gives it a pressed look.
  - The unnamed Settings stop was the credit link at the page's foot. It now
    has a name and a ring, comes last in Tab order and scrolls into view.

**Runtime**
The figures are in `/home/samer/omk-council/claude/round3-perf.txt`. "Opt"
means `--release` without LTO, with debug assertions on so the screenshot
hooks work. Before is main; after is this branch.

| what | before | after |
|---|---|---|
| Release, window visible, oneMK off, 60 s, 2 runs | 0.03 / 0.02 % | 0.03 / 0.03 % |
| Opt, desk with 3 computers, visible, 30 s, 3 runs | 0.23 / 0.23 / 0.20 % | 0.23 / 0.17 / 0.20 % |
| Opt, minimised, 3 runs | – | 0.03 % each |
| Opt, help explainer open, 3 runs | 5.77 / 4.43 / 4.53 % | **0.03 % each** |
| Opt, moving frames, p95, 3 runs | 7.08 / 7.14 / 7.22 ms | 7.09 / 7.12 / 7.13 ms |

**AT-SPI (a real read, Linux)**
- The session's accessibility bus was turned on for the run and then
  restored. A plain `gi.repository.Atspi` client then read the window's tree.
  The dumps are in `round3/atspi-desk.txt`, `round3/atspi-advanced.txt` and
  `round3/atspi-health.txt`.
- **Roles and names as read:**
  - Computers on the desk: toggle buttons, for example "MacBook Air, macOS,
    on your left".
  - Search: an entry.
  - Pages: page tabs, with the current one checked.
  - Position: radio buttons, for example "Position of MacBook Air: Left",
    checked.
  - Number fields: entries, for example "Don't cross near corners, in pixels
    (0: off)".
  - Health: a status bar "2 need attention · 1 pending…", with headings and
    list items.
  - Latency, as text: "Latency to MacBook Air: 4 ms, smooth. Over the last
    2 minutes: lowest…, average…, highest…".
- **What the dump found and this round fixed:**
  - Pending items had ids that collided with Needs you, so Pending read as
    empty.
  - Live Health lines were read twice.
  - The desk card had no words for its latency.
- **Not run:** Orca is installed, but it was not run. Speaking aloud on the
  owner's live desktop wasn't appropriate. No macOS or Windows screen reader
  was run.

## Round 4 (track A)

- **Cards and telemetry:**
  - Short cards ("This computer", the Arrange and Exact place cards, oneMK
    off) end a padding below what they hold. The height is measured from the
    shapes each card actually drew.
  - With fewer than 8 readings, the latency graph is replaced by one line:
    "N readings so far · the graph fills in over 2 minutes".
- **Room:** the coloured wash behind the screens is gone. Each screen has
  only a small, near-neutral pool of light under it.
- **Buttons:** one emphasis system. Volt is for the primary action. Every
  other button, toolbar included, is a quiet solid fill, with no rim or
  outline.
- **Sidebar:** hover is half a step of tone with no edge. The chosen page is
  lighter and has an edge; its shortcut keeps 5.4:1 contrast.
- **Keyboard:**
  - Tab order is Search, the pages, the sidebar's computers and footer, the
    view switch, the toolbar left to right, the computers on the desk, then
    the card. The sidebar and toolbar are made first, on a layer above the
    desk.
  - Settings starts at Updates.
  - The desk's focus ring follows the computer's own outline: a convex hull
    pushed out 5 pt, which never wraps a neighbour.
  - AT-SPI reports the chosen computer as `pressed`. The dump is
    `round4/atspi-desk.txt`, with every state shown, unfiltered.
- **Health:**
  - The cause is in INK and the fix, in INK2, starts with a verb.
  - The pending row is titled "Pending permission", and the passing group
    is called "Passed".
  - Rows have no Recheck buttons of their own; the one Recheck is at the top.
  - The firewall and port rows name the port, and the program or firewall
    when doctor reports it. A test covers this.
- **Copy:**
  - The sidebar and the card show the same newest latency reading.
  - Settings no longer repeats "Ready. You can change shortcuts in
    Settings".
  - The error band is one line with Try again first.
- **No text-only enlargement:** the app has no text-only size setting, so
  there is no capture of one. Zoom scales the whole interface (`s100`–`s200`
  shots). That is a real gap, not a design choice.
- **Runtime, paired before (main) and after:** optimized build, debug hooks
  on, desk with 3 computers, full log in
  `/home/samer/omk-council/claude/round4-perf.txt`.
  - Visible at rest: 0.20 / 0.20 % before, 0.23 / 0.13 % after.
  - Minimised: 0.00 / 0.07 % (after only).
  - Help explainer open: 5.1 % before, 0.00–0.07 % after.
  - Moving frames, 3 runs each: the UI thread's own work per frame (update,
    layout and shapes, no GPU or vsync) went from p50 0.52 / p95 0.63 ms to
    p50 0.15 / p95 0.22 ms. CPU per frame across all threads went from
    0.97 to 0.53 ms. Frame pacing p95 is 7.1 ms in both, set by vsync.
    Most of the saving is the room's light meshes, now gone.

## Round 5 (track A)

- **Accessibility:**
  - "Type on every computer" is a switch. AT-SPI reads it as a toggle
    button, `pressed` while broadcasting (`round5/atspi-desk-broadcasting.txt`).
  - Stop is a plain button, "Stop oneMK: turn off sharing on this computer";
    plain rows no longer read as toggle buttons.
  - The computers' Tab names say "this computer" once.
  - The debug Tab walk names every stop by id, so scrolling no longer leaves
    stops unnamed.
- **Footer and pairing:**
  - The footer says "On · nothing paired yet" or "On · waiting for
    computers" until a computer is actually connected.
  - A pairing match with this computer's own name is called "another
    computer named …".
  - The card names lost packets ("3 packets lost"), apart from stalls.
- **Desk:**
  - With nothing paired, this computer's card is the first step and carries
    the one Volt "Pair a computer"; the sidebar's copy turns quiet.
  - "Options here" replaces a second "This computer" beside this computer's
    own card.
  - A peer card without a graph ends under its actions.
  - The drag hint shows once, for 12 seconds.
- **Layout:**
  - The three cards share one height.
  - The compact tabs have one chosen look (the underline is gone).
  - An open profile comes first, and the save row steps aside, so its
    options are never cut. The list always ends on a whole row.
- **Keys:** a set quick setup is a lighter capsule with semibold words and a
  Volt tick.
- **Health:**
  - A permission appears once (Pending), not also under Needs you.
  - Status markers are one family: a round badge plus a word (problem,
    warning, waiting, passed).
  - Screen readers hear each cause together with its fix.
  - Typing is described in plain words.
- **Palette:** only the typed characters stand out, in ink and semibold
  (no Volt text).
- **Controls:** pressed looks that show on dark surfaces. A switch's knob
  stretches, an unchosen segment lifts more than on hover, and a sidebar
  row takes the chosen tone without its edge.
- **Evidence:**
  - State sheets: `round5/sheet-state-*.png`.
  - Sheet open → close strips, normal and reduced motion:
    `sheet-strip-sheet-rm0/1.png`.
  - The focus ring moving: `sheet-strip-ring.png`.
  - 1:1 crops at 1× and 2× of Desk, Settings and Health:
    `crop-1to1-*.png`.
  - Runtime: `round5/perf.txt`.
- **Honest gap:** the screenshot harness's simulated pointer press
  (`ONE_MK_APP_PRESS`) does not register as a press. Hover drops instead,
  so the `*-pressed` stills show no pressed state. The pressed looks are in
  the code and covered by `pressed_is_its_own_look`. They are not
  demonstrated in screenshots.
- **Runtime:** the machine was loaded (load average 8). Release build,
  visible, at rest: 0.08 % of a core. Help explainer open: 5.97 % before,
  0.03 % after. Minimised: 0.03 %. UI-thread time per frame, p95: before
  0.76–0.97 ms, after 0.30–0.74 ms.
