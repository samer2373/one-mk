# Round 4 · codex · Track B
Branch: `council/codex-r4`; worktree: `/home/samer/.t3/worktrees/one-mk/t3-d6ededa0`.
Base: Claude `09639b22`, subsequently rebased onto current `council/claude` `429b3ecf`; no push, PR, merge or tag.
Design commits: `449e6673`, `48321265`, `571e50a3`, `9bc78a99`; native field values: `8e1aa81d`; focus cleanup: `e69e9a8f`.
Evidence/script commits: `c97a78f6`, `40a40d6c`; final report is committed separately.
Shots: `/home/samer/omk-council/codex/round4/`; `before/`, `iteration1/`, `iteration2/`, `final/`, `guide/`, `evidence/`.
Final index (19 PNGs): 04, 10, 19/19b/19c, 30, 33, 36, 40, 44, 47/47b, 48, 50, 71, 93/93b, 95, 112.
Guide: 13 captured stages; final design PNGs and guide contact sheets reviewed; native walkthrough inspected at representative stops.
Files 04/33/93: oversized scene-first split → named send/drop chips, optional 140pt scene, transfer rows, Clipboard, then History before preferences.
Files 93b: task content delayed → first transfer, clipboard summary, retention switch and privacy line in the compact first viewport; Show all reveals every transfer.
History: external/redundant status → status inside its card, Clear disabled while off; two-line 36pt transfer rows and thin progress rules.
Keys 44/50: separated headings and oversized editor → in-card headings/close, compact stage, padded selected cap and one bounded remap legend.
Mouse 47/48: clipped selected model → full-model camera with selected-part tint; projected-anchor leaders retained, controls earlier, pointer controls reachable by scrolling (47b).
Layout 30/36: fixed-height Position waste → measured content; distinct active tabs; network/profile lines wrap independently.
Sheets 10/19: truncated recipient/explanation → width measured against buttons and wrapped copy; small and 2x chooser fixtures included.
Updates 112: password/restart explanation cuts at 2x → measured wrapped note and content-sized card.
Transfers 30/40/71: overlay inside scene → reserved upper band at all widths; Wallpaper 95 and guide remain visually unchanged.
Iterations: baseline → task-first Files; first pass → compact editors/measured Layout; second pass → complete transfer list/sheet order; final review → corrected mouse camera arguments.
Shared scope: no style.rs edits; widgets.rs only publishes text-field values plus opt-in fixture cfg; other cfg-only changes enable release evidence.
Gates: Linux check, Windows MSVC check, Apple aarch64 check, all-target clippy with design-evidence and -D warnings PASS; app tests 137 PASS; diff --check PASS.
Gate logs: `evidence/gates/`; private target-codex, -j 3, no fmt/fat LTO; every GUI run uses flock -w 600 -o and a throwaway --home.
Keyboard: 165 real XTest Tab captures; AT-SPI records every focus identity; no unnamed stops after removing board/mouse drag-container focus.
Body ring candidates present in every Files/Keys/Layout stop; representative recipient, cap, lower Held-after field and profile field visually checked, including scroll-into-view.
Inherited shell exception: Search has native focus but no visible ring (Files 01/24/47, Keys 01/34, Layout 01/21/41); Track A must fix before release.
AT-SPI: real isolated D-Bus/GI dump, unfiltered states/interfaces; Held-after Text value 200, mouse speed values 1, clipboard limit 25, empty app/gesture/profile fields exposed.
AT limitations: custom Search and Test-it entries lack Text interfaces; no spoken VoiceOver/NVDA review; keyboard rings heuristic is supporting evidence, not a conformance proof.
Runtime: visible/minimized/help each 0.02s CPU over 60s (0.033% of one core); optimized release design-evidence, LTO=false/codegen-units=16, source e69e9a8f; hash in evidence/runtime/runtime.json.
Runtime method: Files visible, window-manager-verified minimized and help-open, 60 seconds each after settling; process CPU at 100Hz, Xvfb/Mesa llvmpipe software renderer.
Runtime scope: not stock release flags or hardware-GPU evidence; no moving-frame/Desk regression claim (Track A owns the paired Desk packet).
Self-scores / reasons: Hierarchy 8.8 (task-first); Geometry 8.6 (measured groups, partial keyboard context remains).
Typography 8.5 (wrapped names/notes, difficult scripts unverified); Colour 8.8 (quiet hierarchy, no new semantic colours).
Surfaces 8.8 (solid content, inherited floating system); Controls 8.5 (consistent chips, native values, model focus cleanup).
Motion 8.0 provisional (retained bounded settling, not independently profiled); Accessibility 7.8 (real traversal, shell Search/Text gaps remain).
Copy/recovery 8.5 (explicit send/privacy/status, offline-recipient hint could be more actionable); Rendering/runtime 8.0 provisional (software-renderer packet only).
Weighted self-assessment 8.47 before deductions; inherited invisible Search focus triggers -1.0 → provisional 7.47 overall, not 9.5 or release approval.
Remaining: far keyboard edges intentionally crop around the padded selected key; narrow mouse settings need scrolling; whole-UI zoom exists, text-only enlargement does not.
To reach 10: reconcile Track A focus/semantics, independent AT/platform/DPI review, hardware runtime/motion profiling, and real file-picker/physical transfer recovery verification.
