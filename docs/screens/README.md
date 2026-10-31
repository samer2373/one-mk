# Window screenshots — wave 5

All 73 PNGs in this directory were regenerated and visually inspected on
2026-10-31. These are **design records, not native-platform acceptance evidence**.
They show the current debug window at its default 980×660 size unless a filename
specifies small (820×540) or HiDPI. Existing filenames are retained for links.

## Provenance and safety

Captures used synthetic status, history, input-rule and diagnostic fixtures on a
private Xvfb display, with a clean environment and disposable HOME,
XDG_RUNTIME_DIR and ONE_MK_HOME. No owner session, clipboard, configuration or
running service was used. No discovery or real pairing process was started.
Names, transfers, latency samples and clipboard entries are illustrative.
In particular, filenames containing `two-daemons` are retained historical names:
these new images are **not** evidence from two live daemons. The fixture-only
debug path does not start a tray or read live clipboard history.

The final inspection included every image in contact sheets, plus full-size
checks of history, pairing fallback, configured broadcast stop and the hovered
latency definition. Minimum-size Files keeps a compact scene and a scrollable
history; default-size history shows nearly three rows. Focus/drag filenames
describe scripted visual states, not a new keyboard or drag/drop acceptance run.
Wallpaper playback, native Windows/macOS and portal permission dialogs are not
represented. Do not run old screenshot scripts against a normal desktop session.

## Inventory

| File | State illustrated |
|---|---|
| [01-intro.png](01-intro.png) | Introductory desk animation and first-use guidance. |
| [03-desk-connected.png](03-desk-connected.png) | Connected desk with service and broadcast controls. |
| [04-files-tab.png](04-files-tab.png) | Clipboard formats, fetch permission and history disabled. |
| [06-small-window.png](06-small-window.png) | Desk controls at minimum window size. |
| [08-nothing-paired.png](08-nothing-paired.png) | Running service with no paired computers. |
| [09-not-running.png](09-not-running.png) | Service stopped; window remains available. |
| [10-pairing-code.png](10-pairing-code.png) | Pairing code confirmation. |
| [11-pairing-searching.png](11-pairing-searching.png) | Pairing search before the delayed fallback hint. |
| [12-peer-offline.png](12-peer-offline.png) | One offline computer and one connected computer above. |
| [13-this-computer-selected.png](13-this-computer-selected.png) | This computer selected in Desk. |
| [14-mid-drag.png](14-mid-drag.png) | Scripted computer drag position. |
| [15-keyboard-focus.png](15-keyboard-focus.png) | Scripted Desk focus state. |
| [16-error-toast.png](16-error-toast.png) | Failed-operation toast without losing the desk. |
| [17-many-long-names.png](17-many-long-names.png) | Many computers, long names, Unicode and a top placement. |
| [18-many-advanced.png](18-many-advanced.png) | Advanced settings with several computers. |
| [19-drop-chooser.png](19-drop-chooser.png) | File destination chooser for multiple dropped files. |
| [21-controlled-small.png](21-controlled-small.png) | Another computer controls this one, at minimum size. |
| [22-paired.png](22-paired.png) | Successful pairing confirmation. |
| [23-reduced-motion-intro.png](23-reduced-motion-intro.png) | Intro with reduced motion. |
| [24-layout-two-daemons.png](24-layout-two-daemons.png) | Two-peer Layout fixture, not two live daemons. |
| [25-health-two-daemons.png](25-health-two-daemons.png) | Health fixture for a connected desk. |
| [26-advanced-defaults.png](26-advanced-defaults.png) | Default Advanced controls and complete shortcut list. |
| [27-health-not-running.png](27-health-not-running.png) | Health when the service is stopped. |
| [28-desk-quick-actions-transfer.png](28-desk-quick-actions-transfer.png) | Desk quick actions while transfers are in progress. |
| [29-layout-offset-multi-monitor.png](29-layout-offset-multi-monitor.png) | Explicit offsets and multiple monitors. |
| [30-layout-edges-dont-meet.png](30-layout-edges-dont-meet.png) | Layout warning when screen edges do not touch. |
| [31-layout-mid-drag.png](31-layout-mid-drag.png) | Scripted drag state in Layout. |
| [32-layout-keyboard-focus.png](32-layout-keyboard-focus.png) | Scripted Layout focus state. |
| [33-files-transfers-clipboard.png](33-files-transfers-clipboard.png) | Transfer progress, clipboard policy and history switch. |
| [34-health-failing.png](34-health-failing.png) | Diagnostic failures and actionable Health details. |
| [35-advanced-settings.png](35-advanced-settings.png) | Customized Advanced settings and current shortcuts. |
| [36-small-layout.png](36-small-layout.png) | Multi-monitor Layout at minimum size. |
| [39-small-advanced.png](39-small-advanced.png) | Advanced controls at minimum size. |
| [40-hidpi-desk.png](40-hidpi-desk.png) | Desk with HiDPI scaling. |
| [42-layout-nothing-paired.png](42-layout-nothing-paired.png) | Layout with no paired computers. |
| [43-keys-keyboard.png](43-keys-keyboard.png) | Keyboard rule editor. |
| [44-keys-edit-and-test.png](44-keys-edit-and-test.png) | Rule editing and key test panel. |
| [45-keys-press-a-key-small.png](45-keys-press-a-key-small.png) | Key capture prompt at minimum size. |
| [46-keys-list-search.png](46-keys-list-search.png) | Searchable keyboard rules. |
| [47-keys-mouse.png](47-keys-mouse.png) | Mouse tuning overview with the mouse-mode section below. |
| [48-keys-mouse-small.png](48-keys-mouse-small.png) | Scrollable mouse tuning at minimum size. |
| [49-keys-problems-long-name.png](49-keys-problems-long-name.png) | Rule validation problems and long computer names. |
| [50-keys-focus.png](50-keys-focus.png) | Scripted focus in the input editor. |
| [51-keys-nothing-set.png](51-keys-nothing-set.png) | Empty input-rule state. |
| [52-broadcasting-many.png](52-broadcasting-many.png) | Broadcast to many names; keyboard-only stop fallback. |
| [52-layout-suggestion.png](52-layout-suggestion.png) | Learned placement suggestion awaiting consent. |
| [53-broadcasting-small.png](53-broadcasting-small.png) | Disabled broadcast-stop click and service stop at minimum size. |
| [53-layout-suggestion-small.png](53-layout-suggestion-small.png) | Placement suggestion at minimum size. |
| [70-latency-live-two-daemons.png](70-latency-live-two-daemons.png) | Latest keep-alive RTT fixture; historical filename, not live measurement. |
| [71-latency-smooth.png](71-latency-smooth.png) | Smooth scripted network RTT with historical summary. |
| [72-latency-stalls.png](72-latency-stalls.png) | Scripted RTT spikes. |
| [73-latency-loss.png](73-latency-loss.png) | Scripted missing replies. |
| [74-latency-slow.png](74-latency-slow.png) | Sustained slow scripted link. |
| [75-latency-offline.png](75-latency-offline.png) | Offline peer has no current RTT. |
| [76-latency-measuring.png](76-latency-measuring.png) | Too few samples for a historical quality assessment. |
| [77-latency-narrow.png](77-latency-narrow.png) | RTT graph at minimum size. |
| [78-latency-hidpi.png](78-latency-hidpi.png) | HiDPI graph and loss state. |
| [79-latency-health.png](79-latency-health.png) | Per-peer RTT summaries in Health. |
| [80-latency-health-narrow.png](80-latency-health-narrow.png) | Health graphs at minimum size. |
| [81-latency-health-long-names.png](81-latency-health-long-names.png) | Health with long and Unicode names. |
| [82-latency-older-daemon.png](82-latency-older-daemon.png) | Older-daemon fixture without link statistics; restart guidance. |
| [83-pairing-manual-fallback.png](83-pairing-manual-fallback.png) | Discovery timeout: invitation/join and actual manual pairing port. |
| [84-history-on.png](84-history-on.png) | Memory-only history enabled with masked entries. |
| [85-history-revealed.png](85-history-revealed.png) | One revealed synthetic multiline preview with separators and ellipsis. |
| [86-history-small.png](86-history-small.png) | Compact Files scene and scrollable history at minimum size. |
| [87-top-layout.png](87-top-layout.png) | Computer above remains fully inside Layout scene bounds. |
| [88-pairing-waiting.png](88-pairing-waiting.png) | Waiting for confirmation on the other computer. |
| [89-pairing-ended.png](89-pairing-ended.png) | Pairing ended without matching codes; retry available. |
| [90-advanced-shortcuts.png](90-advanced-shortcuts.png) | Shared shortcut catalog including Broadcast and Fetch. |
| [91-mouse-gestures.png](91-mouse-gestures.png) | Scrolled mouse modes: autoscroll switch and formatted gesture command. |
| [92-advanced-typing.png](92-advanced-typing.png) | Typing panel marks letter and auto modes unavailable. |
| [93-broadcast-configured-shortcut.png](93-broadcast-configured-shortcut.png) | Daemon-provided custom Ctrl+Alt+F8 stop hint, not a guessed default. |
| [94-latency-definition.png](94-latency-definition.png) | Hovered graph tooltip defines RTT, latest sample and historical summaries. |
