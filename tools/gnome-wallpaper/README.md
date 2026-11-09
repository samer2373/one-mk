# GNOME wallpaper qualification tools

Developer tooling for the optional GNOME Shell wallpaper component
(`extensions/gnome/`). Nothing here ships in a package. Everything runs only inside
a private, headless GNOME Shell (own session and system D-Bus, temporary `HOME`,
no access to your desktop, extensions, settings or display manager).

| File | Purpose |
| --- | --- |
| `isolated-session.sh` | Starts/stops the private Shell (`start [N]`, `run CMD...`, `restart`, `stop`). |
| `qualify-extension-setup.py` | Install, log-out notice, discovery, separate enable and uninstall through the normal CLI. |
| `qualify-production-video.py` | Production video check: screenshots, lifecycle and CPU, run through `isolated-session.sh run`. |
| `test_production_video.mjs` | `node --test`: extension admission and cleanup logic without a Shell. |
| `test_presentation_cleanup.py` | Run in CI (`wallpaper-import-gates`): the qualification runner always stops its private session. No GNOME needed. |
| `qualify-m1-presentation.py`, `m1-risk-probe.py`, `m1a-*.py`, `js/` | Older laboratory presentation/stress gates (need laboratory builds). |
| `client/`, `ext/` | Helpers and the test-only harness extension that grants Eval inside the private session. |
| `baseline-shell.log` | Expected Shell warnings; a changed warning is a failure. |

See [GNOME-WAYLAND-VIDEO.md](../../docs/GNOME-WAYLAND-VIDEO.md) and
[GNOME-WAYLAND-PRODUCTION-SETUP.md](../../docs/GNOME-WAYLAND-PRODUCTION-SETUP.md).
The earlier spike write-ups and raw evidence were removed from the tree and
remain in the repository history.
