# Design review: the window and the tray

An independent review of the one·mk window (`crates/app`) and tray (`crates/tray`) after the wave-1
features (Desk, Layout, Clipboard & files, Keyboard, Health, Advanced), against
[the brand guideline](brand/README.md) and general UI craft. It used the `impeccable` critique, audit and
polish references. They are written for web and mobile, so the detector and browser steps don't apply to an
egui app; this review is a design review only. Screenshots are from `scripts/ui-shots.sh` on GNOME Wayland
at a 1.25 scale. Filenames refer to the set in [`screens/`](screens/); "(before)" means the set this review
started from, in git at `e72766f`.

**The promise being checked:** setup takes about a minute, and it is easier than Deskflow. For each tab,
the questions were what it is for, what the main action is, and whether someone with one paired computer
sees anything they don't need.

## Summary

| severity | found | fixed | left |
|---|---|---|---|
| blocker | 0 | – | – |
| major | 8 | 8 | 0 |
| minor | 13 | 10 | 3 |
| nit | 6 | 0 | 6 |

There were no blockers: every task can be finished. The major findings were all about information
architecture and progressive disclosure, not looks. The visual system (tokens, type scale, one amber
action, status shown by shape) is sound, and this review changed it in only one place (disabled labels).

## Major findings (all fixed)

**M1. Six tabs, and "Keyboard" was the wrong name for what it held.** *(before: 05-keyboard-tab,
18-many-keyboard)* The Keyboard tab held one setting per computer (swapping Ctrl and Cmd), and its default,
Auto, is already right for nearly everyone. Meanwhile the *keyboard shortcuts* were in Advanced. So a person
looking for shortcuts opens Keyboard and doesn't find them, and the bar had six choices where five do.
**Fix:** removed the tab. A "Ctrl and Cmd" card in Advanced, beside Keyboard shortcuts, has the same
controls for each computer and one line saying what Auto does. In a narrow window, the choices shorten
to Auto / Swap / No swap rather than being cut off (*39*). `ONE_MK_APP_TAB=keyboard` now opens Advanced. *(after: 03-desk-connected, 26-advanced-defaults, 18-many-advanced)*

**M2. The Desk showed controls that do nothing yet.** *(before: 08-nothing-paired, 09-not-running)* With
nothing paired, or with one·mk off, the Desk still showed "Keep the pointer here", "Gaming mode" and "Lock
all". So the very first screen had four switches, three of them meaningless. Gaming mode is also a setting
you set once, sitting among everyday actions.
**Fix:** until a computer is paired (and while one·mk is off), only "Start at login" shows. Gaming mode
moved to a "Games" card in Advanced. With computers paired, the Desk column is now "Start at login", "Keep
the pointer here" and "Lock every screen". *(after: 08-nothing-paired, 09-not-running, 03-desk-connected)*

**M3. Health warnings looked like empty radio buttons.** *(before: 25-health-two-daemons, 34-health-failing)*
A warning was a hollow ring, the shape of an unselected radio button and close to "nothing here". From its
icon alone it read as the least important row, not the second most.
**Fix:** a warning triangle (outline, bar and dot) in ink. Problems keep the filled red dot, passes the
tick, and skipped checks the dash. *(after: 25, 34)*

**M4. Health told window users to use the terminal for what the window does itself.** *(before:
27-health-not-running)* When one·mk was off, the "Program" row offered "Copy command: `one-mk run`", and
the "Paired computers" row offered `one-mk pair`, while the window's own top bar had "Start one·mk" and
"Pair a computer".
**Fix:** these two checks now show the window's own button ("Start one·mk", "Pair a computer"), with "Choose
Start one·mk." as the fix text. All other checks keep Copy command. To make this possible, `view::Check`
now carries doctor's `id`. *(after: 27)*

**M5. On the Layout plan, nothing said whose second monitor was whose.** *(before: 29, 30, 41)* A
computer's other monitors were labelled only "1920 × 1080". Only the faint wallpaper tint linked them to
their computer, so a first-time user couldn't tell that the top-middle one was this computer's and the
bottom-left one was Studio PC's.
**Fix:** when there is room, each extra monitor shows its computer's name above its size. *(after: 29, 30)*

**M6. "Line up" offered three controls for one decision, two of them dead.** *(before: 24-layout-two-daemons)*
In the default state, Stretch to fit is on, so the Top/Middle/Bottom choices and the Offset field showed
disabled, with "—" in the field. A newcomer saw three ways to answer the same question, and two of them
didn't respond.
**Fix:** while Stretch to fit is on, the column shows only the switch and one sentence: what stretching
does, and that turning it off lines the two up like monitors side by side. The rest appears when it's off.
"1:1" became "at the same scale". *(after: 24, 29)*

**M7. The window and the tray used different words for the same things.** *(tray, no screenshot)* The
window said "Keep the pointer here", "Above/Below", "one·mk isn't running" and "Lock every screen". The tray
said "Lock pointer here", "Top/Bottom", "one-mk is not running" and "Lock all computers". With nothing
paired, the tray's only advice was "Run `one-mk pair` on both computers", a terminal command, although the
window pairs in one click.
**Fix:** the tray now uses the window's words. With nothing paired, it shows an item "Pair a computer…"
that opens the window (the existing Open action, so no new command). "Waiting for your computers" now
matches the window too, and the window's "Switch here" became "Switch back here", as in the tray.

**M8. A security-relevant switch sat in the wrong group.** *(before: 04-files-tab, 33)* "Others can get
this clipboard" lets any paired computer read this clipboard, including copied passwords. It sat in the
files column, under "Notify when files arrive", instead of in the Clipboard group it belongs to.
**Fix:** it is now the fourth switch under Clipboard. The size limit moved to the right column as "Largest
clipboard ___ MB", a label that makes sense outside the group. *(after: 04, 33)*

## Minor findings

| # | screenshot (before) | problem | fix | status |
|---|---|---|---|---|
| m1 | 35, 39 | The "Crossing edges" card opened with a line repeating its own controls in daemon words ("dwell 300 ms, hold shift, fullscreen guard; left ed…"), cut off. | Removed. The note about per-edge rules stays when there are some. | fixed |
| m2 | 26, 35 | Jargon in Advanced: "Dead corners", "Hold a key", "Hooks". | "Don't cross near corners" (hover explains why), "Hold a key to cross", "Run commands". | fixed |
| m3 | 26 | Cards had fixed heights: the empty Hooks card was mostly blank, and the order didn't follow how often things change. | Cards fit their contents. Left column: crossing, power, games. Right: shortcuts, Ctrl and Cmd, commands. | fixed |
| m4 | 24, 29 | The legend said "Wall". Dashes meant both a wall and an outline of unknown size (Gaming PC, offline). | Legend: "The pointer stops here". Unknown-size outlines are solid, so dashes mean only one thing. | fixed |
| m5 | 23-reduced-motion-intro | With reduced motion, the panel cells still arrived one after another (0.25–0.33 s), so the third column was empty at 0.3 s. | The stagger uses `motion::space`, which is 0 under reduced motion: the cells fade in together. | fixed |
| m6 | 09-not-running | Disabled tab and segment labels were 2.73:1. WCAG exempts inactive controls, but these labels tell you what the app has. | 4.1:1 (`mix(INK2, RAISED, 0.3)`), still clearly quieter than INK2 (7:1). | fixed |
| m7 | 30 | A profile's Wi-Fi row had a bare "Off" button next to a Wi-Fi field. | "Turn off". | fixed |
| m8 | 26 | The px and ms fields in Crossing edges didn't line up (4 px apart), because the units differ in width. | A shared unit column. | fixed |
| m9 | 24, 42 | "Line up" empty-state copy used "1:1". | "side by side like two monitors". | fixed |
| m10 | – | `ui-shots.sh` started its daemons with `--backend` before `run`, and used a fixed work folder that other sessions running it could share. | `run --backend null`; a work folder per run, removed on exit; `-j 3`. | fixed |
| m11 | 25 | The live box said "sharing works: the desktop allowed it" while doctor's Permissions check said "you have not allowed one-mk yet". | Not fixed. Doctor runs in its own process and asks the portal separately. The test daemons also use the null backend, so this may only be a screenshot artefact. Reconciling the two needs a change in `crates/one-mk`. | left |
| m12 | 30 | "Delete" removes a saved profile without confirmation or undo. | Not fixed. The stakes are low (save it again), and a confirm sheet would add a step to a rare task. | left |
| m13 | 29 | "In use" is drawn as a disabled button, so it looks pressable. | Not fixed. It reads correctly beside "Use" on the other rows; a chip would be a new component. | left |

## Nits (left as they are)

- **n1** *(25)* Health says "Copy" for plain-text fixes and "Copy command" for commands. Different payloads,
  so different words; fine.
- **n2** *(08, 13)* This computer's inspector shows "ID 3ff4-d244". It's jargon, but it's what support and
  `one-mk` output refer to.
- **n3** *(06, 21)* In a narrow window, the "Clipboard" label before the two icon buttons drops out. The
  buttons keep their tooltips and accessible names ("Send this clipboard to …").
- **n4** *(26, 35)* The shortcut notes "(not with the null input backend)" and "through the GlobalShortcuts
  portal" are the daemon's own words. The first appears only with the test backend; "portal" is jargon, but
  it's the word GNOME's own settings use.
- **n5** *(tray)* The tray always lists "Take back control", even when nothing controls this computer. Left
  on purpose: it's the way out when the status the tray shows is stale.
- **n6** *(22-paired)* The screenshot often catches "Found a computer · Connecting…" instead of "Paired".
  This is screenshot timing, not a bug.

## Checked and fine

- **Hierarchy.** Each tab has one job: Desk (where things are, cross over), Layout (line edges up),
  Clipboard & files, Health (what's wrong and the fix), Advanced (set once). There is one amber action in the
  top bar, and it follows the state: Pair a computer → Pair another (quiet) → Start one·mk → Take back.
- **Colour (brand).** Amber appears only for the main action, crossing edges, the pairing outline,
  progress and focus. Switches show "on" in ink, not amber. Status is shown by shape (dot or ring), never by
  colour alone.
- **Type.** Geist 600 at 15 for titles, 400 at 13 for body, 12 for meta; Geist Mono for numbers, keys and
  sizes. Matches the guideline.
- **Spacing and grid.** The panel is 3 columns with 20 pt padding; rows are 36 pt (30 in the Clipboard
  list); cards have 18 pt insets. These are consistent across tabs. The column dividers move between tabs
  (Layout 30/60 %, Files 34/63 %), which is noticeable but follows the content.
- **Icons at 1x and 1.5x** *(03, 40)*. The clipboard send/get icons, the refresh icon and the Health glyphs
  stay legible. The lockup uses the pixel-fitted mark.
- **Targets.** Buttons are 28–34 pt tall, icon buttons 28 pt, and the measure button 24 pt. Segment cells
  are 22 pt tall but 50 pt or more wide, which passes WCAG 2.5.8 through its spacing exception.
- **Long names, many computers, other scripts** *(17, 18, 19)*. Names are shortened with an ellipsis and
  the full name is on hover. Cyrillic and CJK names load fallback fonts. The drop chooser lists every
  computer.
- **Motion.** Every animation goes through `motion::space` or `motion::fade`. With reduced motion, spatial
  movement snaps and fades last at most 0.12 s; m5 fixed the one stagger that didn't follow this. Idle
  windows ask for no frames.
- **Keyboard.** Tab reaches every control. Arrows choose within segments and tabs. On the plan, arrows move
  the focused computer along its edge (Shift for larger steps). An open sheet locks the controls behind it
  and focuses "They don't match" first, never the confirming button. The focus ring is amber, 2 pt, outside
  the control (*15, 32*).
- **Screen readers.** eframe is built with the `accesskit` feature. Every painted control sets a role,
  name and state: switches, radio segments, tabs, buttons (with fuller names where the visible label is
  short), and fields. Headline, toast and Health summary are live regions. Devices on the desk and the plan
  announce where they are, and on the plan, that arrows move them.

## Contrast

These were sampled from the screenshots: the most contrasting pixel in the region against the region's
most common colour, at 1.25x. "After" was measured on the committed PNGs; their palette keeps these
colours exact.

| what | before | after |
|---|---|---|
| Body sub-text (INK2 on panel) | 7.30:1 | 7.30:1 |
| Field placeholder ("Name this layout") | 6.34:1 | 6.34:1 |
| Disabled tab label (one·mk off) | **2.76:1** | 4.10:1 |
| Disabled "Lock all" label | 6.41:1 | (hidden while off) |
| Focus ring (amber) against the raised track | 9.84:1 | 9.84:1 |
| Health "Warning" word | 7.62:1 | 7.62:1 |
| Warning glyph (ink) on ground | ring, 16:1 but read as a radio | triangle, 16:1 |
| "Offline" on the plan | 7.30:1 | 7.30:1 |
| Second monitor's size label on its wallpaper | 6.70:1 | 6.70:1 |
| Second monitor's owner name (new) | – | 6.54:1 |
| Legend text | 7.62:1 | 7.62:1 |

In code, the field placeholder uses `OFF` (4.3:1). egui draws placeholders as faded ink instead, which
measures 6.3:1, so no change was needed.

## Screenshots

`scripts/ui-shots.sh docs/screens` regenerates the committed set. There are now 35 shots, down from 42:
the Keyboard-tab shots went with the tab (18 now shows Advanced with five computers), and near-duplicates
were dropped (02 probe, 07 large window, 20 many/small, 37 small files, 38 small health, 41 hidpi layout).
For docs/screens, the script saves 256-colour PNGs through libimagequant. That's 3.5 MB instead of
11 MB, with no banding on the desk's gradients; a plain octree quantiser was tried first and banded. Any
other output folder gets exact PNGs. The script also gives each run its own work folder, starts its
daemons with `run --backend null`, and builds with `-j 3`.

## Not verified

- A real tray host. The tray's menu model is unit-tested, but the menus weren't seen in GNOME
  AppIndicator, the Windows notification area or the macOS menu bar.
- Real Windows and macOS rendering. Clippy passes for `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`,
  but the window wasn't seen on those systems (font hinting, scale factors, the macOS Cmd glyphs in
  shortcuts).
- A screen reader actually speaking the tree (Orca, NVDA, VoiceOver). The roles and names were read from
  the code, not heard.
- Keyboard-only use beyond the focus screenshots and reading the code.

## Keys & mouse (plan 4B)

The sixth tab, reviewed the same way from `scripts/ui-shots.sh` (shots 43 to 51, plus 03 and 06 for the
six-tab bar at full and narrow width). Its one job: change what keys and buttons do on the other computers,
and try it before it is used. One amber action per area: "Press the new key" in the key editor, "Press a key
or shortcut" for a mouse button.

| # | screenshot | problem | fix |
|---|---|---|---|
| k1 | 43 (first draft) | Punctuation keys showed their names cut off ("Mi…", "Back…" for `\`). | Keys show their character (`-`, `[`, `\`); long legends ("Backspace", "Super") step down a size before they are cut. |
| k2 | 43 (first draft) | At the default size the rule list and the Test it box were below the fold. | In a wide window the quick setups stand in a column beside the keyboard; the cards below start on screen. |
| k3 | 45 (first draft) | At 760 × 540 a clicked key's editor opened below the fold, unseen. | Opening the editor (or a mouse button's choices) scrolls it into view; no animation with reduced motion. |
| k4 | 45 (first draft) | The "Press the key…" box had a fixed height; its note ran out of it. | The box is as tall as its text; the full list of keys that can't be pressed is in the hover text and the accessible name. |
| k5 | 49 | Problems used the daemon's table names ("input.keymap.rule #6"). | "Key rule 6", "Button rule 1". A long computer name in **For:** is cut with an ellipsis. |
| k6 | 48 | At 760 wide, "As this computer", "Mouse wheel" and "Forward (browser)" were cut. | "Like here", "Wheel", "Go forward"; the accessible names keep the full words. |
| k7 | 46 | The search list stayed seven rows tall for two results. | It is as tall as the results (three rows at least). |
| k8 | first save | Saving a rule from the window wrote the settings folder as the rule's target (`--home` came after `--`). | The folder goes before the command; caught by looking at the saved result, now checked on every shot run. |
| k9 | 50 | The desk's focus hook moved the keyboard off the key in the screenshot. | The hook leaves this tab alone. |

Contrast, sampled from the committed PNGs (most contrasting pixel against the region's most common colour):

| what | ratio |
|---|---|
| Key outline against the panel / against the key's fill | 3.74:1 / 3.08:1 (was 3.18 / 2.62 before `KEY_EDGE` was lightened) |
| Key legend (INK2 on the key) | 6.02:1 |
| A changed key's target (INK on the lit key) | 11.39:1 |
| Focus ring and selected key (amber on the panel) | 10.24:1 |
| Quick setup chip outline | 3.74:1 |
| Test box placeholder | 7.62:1 (was 4.31:1 in OFF; now INK2) |
| Acceleration Off/Curve, disabled | 4.09:1 (as m6: inactive, still readable) |
| Error text and dot | 15.62:1 / 6.44:1 |

Not verified: real key presses into the window on Windows and macOS (only Linux/Wayland screenshots, and
key events were simulated through the debug hooks); a screen reader speaking the keyboard.

## Live latency graph (plan 5B)

The graph is on the Desk tab under the selected computer (shots 70 to 78, 82) and on Health, one row per
computer (79 to 81). Its one job: show a Wi-Fi problem as it happens, quietly. Nothing moves: the line is
redrawn once a second, never animated, so reduced motion changes nothing.

| # | screenshot | problem | fix |
|---|---|---|---|
| l1 | first draft of the Desk card | The inspector had no room left: the card sat below the panel's bottom edge, cut off. | The Desk panel is 58 pt taller (only on the Desk tab, once a computer is paired). The card is compact: the line on the left, the number now and its word beside it, min / avg / max below. |
| l2 | first draft of the loss states | A lost packet was a short amber tick at the foot, where it hid behind the spike of the same moment and could not be told from a slow reading. | A thin line the full height of the graph that ends in a ring, so loss reads apart from slowness by shape, not only by colour. |
| l3 | 82 | After an upgrade, a daemon that still runs the old version reports no round trips: the card said "Measuring…" for ever. | "Restart one·mk to see latency" (Desk) and "Restart one·mk" (Health), also in the accessible text. |
| l4 | 70, 06 | A graph that is only seconds old fills a tenth of its width. | Kept: the time axis is fixed at two minutes so the line doesn't rescale under the eye; "Measuring…" shows until three readings exist. |

Found by tests rather than by eye: a count of lost packets that goes down means another connection, so the
history starts again (a test had it going down by accident, which cleared it); the millisecond formatter
overflowed on the largest round trip a hostile status could carry.

Colour: grey while smooth, amber for "Some stalls" and "Slow". Amber is brand-allowed for live state, and the
word beside every line says the same, so the graph is never colour-only; "Slow" also draws the line thicker.

| what | contrast |
|---|---|
| Graph line, grey (INK2 on raised) | 7.01:1 |
| Graph line and word, amber (on raised / panel) | 9.84:1 / 10.24:1 |
| Lost-packet line (60 % amber on raised) | 4.31:1 (3:1 needed) |
| min / avg / max text (INK2 on panel) | 7.30:1 |

Text equivalent (one screen-reader item per graph): "Latency to desk: 4 ms, smooth. Over the last 2 minutes:
lowest 3 ms, average 4 ms, highest 9 ms." plus "2 packets were lost." when they were. Offline: "Latency to
desk: offline." Computed from the same numbers as the drawing (`rtt::Summary::describe`).

Not verified: a screen reader actually speaking it; Windows and macOS rendering; Wi-Fi loss on real hardware
(the loss and slow states come from scripted histories; the live shot 70 is real loopback readings).
