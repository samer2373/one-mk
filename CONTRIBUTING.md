# Contributing to oneMK

Thanks for helping. Bug reports, tests on real hardware (especially Windows,
macOS, KDE and wlroots desktops) and focused pull requests are all welcome.

## Reporting a bug

Use the **Bug report** issue form. The most useful thing you can attach is
the output of `one-mk doctor` from each computer involved, plus the
operating system and desktop (for example "Ubuntu 24.04, GNOME, Wayland").
Security problems go through [SECURITY.md](SECURITY.md), never a public issue.

## Building and testing

You need a stable Rust toolchain.

```sh
cargo build --locked                       # the programs, debug build
cargo test --locked -p one-mk <filter>     # a focused test run while working
cargo test --workspace --locked            # everything, once before a PR
cargo clippy --workspace --locked --all-targets -- -D warnings
```

The release profile uses fat LTO and needs a lot of memory; a debug build
is enough for development and for the packaging checks
(`BIN_DIR=target/debug sh scripts/check-packaging.sh`).

The window screenshots in `docs/screens` come from `scripts/doc-shots.sh` (a private X server, two
paired test daemons and a throwaway home); rerun it when the window changes and look at every
picture before committing.

Some rules that keep the project healthy:

- **Do not run `cargo fmt`.** The code is formatted by hand; match the style
  around your change.
- **Test in a throwaway home.** oneMK installs login items, desktop entries
  and file-manager actions. Run it with `--home "$(mktemp -d)"` (and a
  scratch `HOME`/`XDG_*` when testing install steps) so your own setup is
  untouched.
- **Every fix gets a test** that fails without it, where one is feasible.
- **Be honest about platforms.** Anything not run on a real Windows or Mac
  (or KDE, or wlroots) is labelled experimental or untested, in the code's
  messages and in the docs.
- **Licences.** If you change `Cargo.lock`, run
  `python3 scripts/third-party-notices.py` and commit the updated
  `THIRD-PARTY-NOTICES`; CI checks it, and `cargo deny check` checks the
  licences of new dependencies (`deny.toml`).

## Pull requests

Keep a pull request to one change, explain what it fixes and how you tested
it (which systems, which desktop), and make sure CI is green. By
contributing you agree that your contribution is licensed under the
project's licence, MIT OR Apache-2.0, at the user's choice.
