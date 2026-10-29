# Packaging

How one-mk is packaged for each system, which file is built how and by which
step of `.github/workflows/release.yml`, and what has and has not been tried.

| Format | Made by | Release step | File published | Status |
|---|---|---|---|---|
| plain `.tar.gz` / `.zip` | inline in the workflow | build: "Package" | `one-mk-<target>.tar.gz` (macOS, Linux), `one-mk-x86_64-pc-windows-msvc.zip` | in use since the first release; `install.sh` downloads the Linux `.tar.gz` |
| macOS `.app` and `.dmg` | `scripts/bundle-mac.sh` | build: "Mac app" | `one-mk-<target>.app.zip` and `one-mk-<target>.dmg`, for `aarch64-apple-darwin` and `x86_64-apple-darwin` | released since 0.1.1; signed and notarized when the signing secrets are set; `install.sh` uses the `.app.zip` |
| Windows installer | `installer/one-mk.iss` (Inno Setup) | build: "Windows installer" | `one-mk-x86_64-pc-windows-msvc-setup.exe` | released since 0.1.2; CI installs it silently and runs `one-mk id` from the install |
| Debian `.deb` | `scripts/bundle-deb.sh` | build: "Linux .deb" | `one-mk-x86_64-unknown-linux-gnu.deb` | released since 0.1.1; CI installs it with `dpkg -i` and runs `one-mk id` |
| AppImage | `scripts/package-appimage.sh` + `packaging/linux/` | build: "Linux AppImage" | `one-mk-x86_64-unknown-linux-gnu.AppImage` | AppDir built and run here and in CI; the `.AppImage` file is built in CI only, **not yet released** |
| Flatpak source bundle | `scripts/package-flatpak.sh` + `packaging/flatpak/` | publish: "Homebrew and Flatpak files" | `one-mk-flatpak-sources.tar.gz` | **written, untested** (`flatpak-builder` never run) |
| Homebrew cask and formula | `scripts/package-homebrew.sh` + `packaging/homebrew/` | publish: "Homebrew and Flatpak files" | `one-mk.rb`, `one-mk-source.rb` | **written, untested** (no Mac) |

`<target>` is the Rust target triple. File names carry no version, so
`https://github.com/samer2373/one-mk/releases/latest/download/<file>` always
gets the newest one (that is how `install.sh` works). `SHA256SUMS` in each
release lists the checksums of the files made by the build job.

The scripts never build the programs themselves, so CI and a developer
package exactly the same binaries:

```sh
cargo build --release --locked -p one-mk -p one-mk-tray -p one-mk-app
sh scripts/bundle-deb.sh target/release one-mk-x86_64-unknown-linux-gnu.deb   # Linux, from the checkout
sh scripts/bundle-mac.sh target/release one-mk-aarch64-apple-darwin.app.zip   # macOS, also writes the .dmg
sh scripts/package-appimage.sh      # dist/, see below
sh scripts/check-packaging.sh       # builds and checks the Linux packages in /tmp
```

`bundle-deb.sh` and `bundle-mac.sh` take the release folder and the output
file and must run from the top of the checkout. The `package-*.sh` scripts
write into `dist/` (or `$OUT_DIR`) and, with `TARGET=<triple>`, look in
`target/<triple>/release` (the release workflow builds with `--target`;
`BIN_DIR=` overrides). On a machine short of memory the release profile's fat
LTO can run out of it; for a local check build with
`CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`, or
check a debug build: `BIN_DIR=target/debug sh scripts/check-packaging.sh`.

## One version

The version is `[workspace.package] version` in the root `Cargo.toml`; every
crate says `version.workspace = true`. Nothing else holds a version:

- `sh scripts/version.sh` prints it; the `package-*.sh` scripts use it.
  `bundle-deb.sh`, `bundle-mac.sh` and the workflow's Windows step read the
  first `version = "..."` line of the root `Cargo.toml`, which is the same
  value.
- The release workflow's build job starts with
  `sh scripts/version.sh --check $GITHUB_REF_NAME` on a tag push, so tagging
  `v0.2.0` while `Cargo.toml` says `0.1.11` stops before anything is built. A
  manual run (`workflow_dispatch`) has no tag, skips the check, builds
  everything and publishes nothing.
- To release: change the version, commit, tag `vX.Y.Z`, push the tag.
- `one-mk --version` prints `one-mk <version>` (clap reads it from the crate).
  `one-mk-app` and `one-mk-tray` have no `--version`; they print their usage
  for an unknown option.

## Checking

`scripts/check-packaging.sh` (also the `packaging` job of `ci.yml`, on a debug
build) builds into a scratch folder and checks, installing nothing:

- the `.deb` from `bundle-deb.sh`: the files in it, the control version,
  `one-mk --version` from an unpacked copy, no world-writable or setuid files,
  `desktop-file-validate`;
- the AppDir: `AppRun --version`, the files appimagetool wants at the top,
  `desktop-file-validate`, `appstreamcli validate --no-net`;
- the Flatpak source bundle: unpacked elsewhere, every `path:` resolves and
  matches its `sha256`, no checkout path or placeholder left;
- the Homebrew files, filled in from stand-in `.dmg` files named like the
  release's: no placeholder left, the right checksum, the right file names
  (and `ruby -c` when Ruby is installed).

Optional tools are reported as skipped when missing. The macOS app and the
Windows installer are checked by the release workflow itself (it signs,
installs and runs them).

## macOS `.app` and `.dmg`

`scripts/bundle-mac.sh <release dir> <out.app.zip>` builds `one-mk.app` (the
`Info.plist` is written inline: bundle id `com.one-mk.app`, version from
`Cargo.toml`, `one-mk-app` as the executable, `LSMinimumSystemVersion` 11.0),
zips it with `ditto` and writes the matching `.dmg` (with an `Applications`
link) next to it.

```
one-mk.app/Contents/Info.plist
              MacOS/one-mk-app        the window (CFBundleExecutable)
              MacOS/one-mk            the command line and the service
              MacOS/one-mk-tray       the menu bar icon
              Resources/onemk.icns    from docs/brand/app-icon/macos
```

- The three programs sit side by side, so `one-mk install` (which looks for
  `one-mk-tray` next to itself) registers LaunchAgents that point inside the
  app.
- With `SIGN_IDENTITY` set (the workflow imports the `MACOS_CERT_P12`
  certificate into a throwaway keychain) it is signed with the Developer ID
  and the hardened runtime; with `NOTARY_APPLE_ID`, `NOTARY_PASSWORD` and
  `NOTARY_TEAM_ID` also set it is notarized and stapled, and the `.dmg` is
  signed too. Without the secrets it is signed ad hoc.
- Both architectures are built on the `macos-14` (Apple silicon) runner; the
  Intel build is a cross build (`--target x86_64-apple-darwin`), because the
  `macos-13` Intel runner never started. One `.dmg` per architecture, no
  universal binary.

## Windows installer

`installer/one-mk.iss` is an Inno Setup script. The workflow installs Inno
Setup with Chocolatey and runs
`ISCC.exe /DAppVersion=<version> /DReleaseDir=..\target\<target>\release installer/one-mk.iss`,
which writes `one-mk-x86_64-pc-windows-msvc-setup.exe` at the top of the
checkout; then it installs it silently into a scratch folder and runs
`one-mk.exe id` from there. The installer:

- installs for the current user only, into `%LOCALAPPDATA%\Programs\one-mk`
  (no administrator rights);
- adds a Start menu shortcut to the window, and optionally one on the desktop;
- optionally (a task, on by default) appends the folder to the user's `PATH`
  if that exact entry is not there yet;
- offers to open the window at the end.

It does not open the firewall, run `one-mk install` (start at sign-in, the
*Send to* entries) or take the folder out of `PATH` on uninstall. Start at
sign-in is turned on from the window (or `one-mk install`); `one-mk integrate`
adds the *Send to* entries. The installer is not code-signed, so SmartScreen will warn.

## Debian `.deb`

`scripts/bundle-deb.sh <release dir> <out.deb>` uses `dpkg-deb` directly. The
package (version exactly as in `Cargo.toml`, `amd64`) contains the three
programs in `/usr/bin`, the window's launcher
`/usr/share/applications/one-mk.desktop` (written inline by the script) and
the 256 px icon. It has no maintainer scripts, starts and enables nothing,
and ships no systemd unit or autostart entry: `one-mk install`, run as
yourself, writes the user service, the tray's autostart entry and the hidden
`io.github.onemk.OneMk.desktop` entry the GlobalShortcuts portal needs into
the home folder. It is built on ubuntu-22.04 (glibc 2.35), the oldest
distribution it is meant for.

## AppImage

`scripts/package-appimage.sh` lays out `dist/one-mk.AppDir` (the three
programs, `packaging/linux/one-mk.desktop`, the AppStream metainfo with the
version and the commit date filled in, the hicolor icons, the copyright file
and `packaging/linux/AppRun`), runs `AppRun --version` to prove the layout
works, and then calls `appimagetool` if it can find one (`APPIMAGETOOL=/path`,
or on `PATH`). The output is `one-mk-<arch>-unknown-linux-gnu.AppImage`, named
like the release's other Linux files.

- `AppRun` (`packaging/linux/AppRun`): with no arguments it starts the window;
  `app` and `tray` start the window and the tray explicitly; anything else goes
  to `one-mk`, so `./one-mk-x86_64-unknown-linux-gnu.AppImage pair` works.
- Inside an AppImage the running copy is in a temporary mount. When one-mk is
  started from one, `one-mk install` and `one-mk integrate` therefore write the
  path of the `.AppImage` file (from `$APPIMAGE`), not the mount, into the
  login service and the file manager entries, and no tray entry (the tray is
  not next to that path). Keep the file where it is after installing.
- The workflow downloads appimagetool 1.9.0 and checks it against a pinned
  SHA-256 (`46fdd785...`). That hash is of the file as downloaded while
  writing this, not independently verified; compare it with the project's
  release page before trusting it. After packing, the workflow runs the
  AppImage once (`--appimage-extract-and-run --version`, since the runner has
  no FUSE).
- The `.desktop` file inside names `one-mk-app`; AppImageLauncher and similar
  tools rewrite the `Exec` line to the file.
- Not done: an update mechanism (zsync), and an AppImage for ARM (the script
  builds one if run on aarch64, untried).

## Flatpak (untested)

`packaging/flatpak/io.github.onemk.OneMk.yml.in` is a manifest that repackages
the release's Linux tarball, `one-mk-x86_64-unknown-linux-gnu.tar.gz` (a
`simple` module), so the Rust build does not have to work offline inside the
sandbox. `scripts/package-flatpak.sh [URL_OR_FILE]`
fills it in; with `flatpak-builder` installed it also builds a bundle. Nothing
in it has been run: `flatpak-builder` is not available here. Runtime
`org.freedesktop.Platform//25.08` is the newest at the time of writing;
adjust it to what exists.

**What is published is a source bundle, not the bare manifest.** The script
copies the desktop file, the metainfo and the two icons the manifest installs
into `sources/` next to it (each with its `sha256:`), writes relative `path:`s
only, and packs the folder as `one-mk-flatpak-sources.tar.gz`
(sorted, clamped times, so the same bytes every time). Given a URL, the
tarball source is that URL with its checksum (from a download made by the
script); given a file, the tarball is copied into the bundle too. The release
workflow's publish job makes it from the tarball's release URL after the
release exists, and uploads it; extract it anywhere and run `flatpak-builder` in
the folder (`README.txt` has the command). `check-packaging.sh` builds a
bundle, extracts it in another folder after deleting the original and checks
that every `path:` resolves there and matches its checksum, with no checkout
path or placeholder left. Only that check ran; `flatpak-builder` did not.

Permissions requested, and why:

| Permission | For |
|---|---|
| `--socket=wayland`, `--socket=fallback-x11`, `--share=ipc`, `--device=dri` | the window (egui, OpenGL) |
| `--share=network` | the QUIC link, pairing, and mDNS. This is the host network namespace, so multicast works. one-mk speaks mDNS itself, so `--talk-name=org.freedesktop.Avahi` is **not** needed (add it only if discovery is ever moved to Avahi). |
| `--talk-name=org.freedesktop.portal.*` | InputCapture, RemoteDesktop, GlobalShortcuts, Clipboard, Notification portals |
| `--talk-name=org.freedesktop.Notifications`, `FileManager1` | "files received" notifications and *Show in folder* |
| `--talk-name=org.kde.StatusNotifierWatcher`, `--own-name=org.kde.*` | the tray icon |
| `--filesystem=xdg-download` | received files in `Downloads/one-mk` |

What cannot work (or will not work as it does on the host):

- **Start at login.** `one-mk install` writes into the sandbox's own config
  folder, not the host's `~/.config/systemd` or `~/.config/autostart`. The
  Flatpak way is the Background portal, which one-mk does not call. Until it
  does, the app has to be started by hand or from the desktop's own startup list.
- **File manager entries.** `one-mk integrate` would write to the sandbox's
  data folder, and the entries it writes name a program path that does not
  exist on the host. Not supported in Flatpak.
- **The `one-mk` command on the host** cannot reach a daemon running in the
  sandbox (the control socket is in the sandbox's runtime folder), so
  `one-mk status`, `switch` and `send` from a host terminal do not see it. Use
  `flatpak run --command=one-mk io.github.onemk.OneMk <command>`.
- **Input on X11 and without portals.** Capture and injection on X11 sessions
  and on compositors without the InputCapture portal use backends that need
  access to `/dev/uinput` or the X server's grabs; none of that is opened here.
  On GNOME Wayland the portals do the work (first-run permission dialog as
  usual); how well, in a sandbox, is exactly what is untested.
- The app id (`io.github.onemk.OneMk`) is the same one `one-mk` registers
  with the GlobalShortcuts portal, which is why Flatpak and the hidden
  desktop entry `one-mk install` writes agree.

## Homebrew (untested)

- `packaging/homebrew/one-mk.rb.in` is the cask (the normal way to install by
  Homebrew): it downloads `one-mk-aarch64-apple-darwin.dmg` or
  `one-mk-x86_64-apple-darwin.dmg` from the tag's release, installs
  `one-mk.app`, links `one-mk` onto `PATH`, and on uninstall unloads the
  `com.one-mk.daemon` and `com.one-mk.tray` LaunchAgents.
- `packaging/homebrew/one-mk-source.rb.in` is a formula that builds the three
  programs from the tag's source tarball.
- `scripts/package-homebrew.sh` fills in the version and the checksums (of the
  two `.dmg` files in `dist/` and of the source tarball, `SOURCE_SHA256` or
  `git archive` of `HEAD`). The publish job attaches both files to the
  release; putting them in a tap (`samer2373/homebrew-tap`) is by hand.

## The release workflow

`.github/workflows/release.yml` runs on a `v*` tag push, or by hand
(`workflow_dispatch`, which builds and tests everything but publishes nothing):

1. `build`, one leg per target (`aarch64-apple-darwin` and
   `x86_64-apple-darwin` on `macos-14`, `x86_64-unknown-linux-gnu` on
   `ubuntu-22.04`, `x86_64-pc-windows-msvc` on `windows-latest`): checks the
   tag against `Cargo.toml` (tag pushes only), builds the three programs, makes
   the plain archive, then the platform's files: the signed `.app.zip` and
   `.dmg` on macOS; the installer on Windows (installed and run); the `.deb`
   (installed and run) and the AppImage (run) on Linux. Every `one-mk-*.*`
   file at the top of the checkout is uploaded as one artifact named after the
   target.
2. `publish` (tag pushes only) collects every artifact into `dist/`, writes
   `SHA256SUMS` and the download table of the release notes, creates the
   release, and then writes and attaches the Homebrew cask and formula and the
   Flatpak source bundle (they need the release's own URLs and checksums).

`ci.yml` runs `cargo test` on Linux, macOS and Windows for every push to
`main` and every pull request, and `scripts/check-packaging.sh` on a debug
build on Linux.

## What the merge with the 0.1.11 release dropped

Two packaging efforts met: the released one (`bundle-mac.sh`, `bundle-deb.sh`,
`installer/one-mk.iss`, all built and tested by the release workflow) and an
unreleased one (`scripts/package-*.sh`, `packaging/`). Where both made the same
file, the released, CI-tested one was kept:

| Dropped | Replaced by | Lost with it |
|---|---|---|
| `scripts/package-windows.sh`, `packaging/windows/one-mk.iss`, `packaging/windows/path-entries.pas`, `scripts/check-windows-path.sh` | `installer/one-mk.iss` | an all-users install in Program Files, firewall rules for private networks, running `one-mk install` at setup and `one-mk uninstall` at removal, taking the folder out of `PATH` on uninstall |
| `scripts/package-macos.sh`, `packaging/macos/Info.plist.in` | `scripts/bundle-mac.sh` | nothing of note (the released script also signs and notarizes) |
| `scripts/package-deb.sh`, `packaging/linux/postinst`, `postrm`, `one-mk.1` | `scripts/bundle-deb.sh` | the packaged systemd user unit, tray autostart and hidden app-id entry (`one-mk install` writes them per user instead), the man page, the copyright file, the full hicolor icon set, the AppStream file, `Depends`/`Recommends`, a reproducible build, `~` for pre-release versions |
| `scripts/package-tarball.sh` | the workflow's "Package" step | a reproducible archive (sorted, time-clamped) |

The AppImage, the Flatpak bundle, the Homebrew files and the checks were kept,
renamed to the release's file names (`one-mk-<target>.<ext>`, no version in
the name) and pointed at the released files.

## File manager entries ("Send to <computer>")

Not a package, but installed with one: `one-mk install` and
`one-mk integrate` add them, `one-mk uninstall` and `one-mk integrate --remove`
take them away (the code is `crates/ctl/src/integrate.rs`).

| System | File | Where |
|---|---|---|
| Nautilus | one executable script per entry | `~/.local/share/nautilus/scripts/` |
| Nemo | the same scripts | `~/.local/share/nemo/scripts/` |
| Dolphin | one service-menu `.desktop` with one action per entry | `~/.local/share/kio/servicemenus/one-mk-send.desktop` |
| Explorer | one `.cmd` per entry | `%APPDATA%\Microsoft\Windows\SendTo\` |
| Finder | one Quick Action `.workflow` bundle per entry (`Info.plist` + `document.wflow`, plain files) | `~/Library/Services/` |

- One computer: one entry, **Send to <name>**. None: **Send with one-mk**, which
  fails with "no paired computers". Several: one per computer, plus
  **Send with one-mk** for the first.
- Every entry runs `one-mk send --notify-errors [--to <id>] -- <files>`. The
  computer is named by its **fingerprint** (hex, checked when read), never its
  name, so nothing another computer chose reaches a command line; names only
  appear in menu labels, cleaned like everywhere else (control and bidi
  characters out, `/ \ : * ? " < > |` replaced, length bounded; duplicates told
  apart by the first four characters of the id). Files come after `--`, so a
  file called `-rf` is a file. Shell words are single-quoted; desktop-entry and
  batch quoting follow their own rules (`%` doubled in both).
- They follow `pair` and `forget`: both call `integrate::refresh`, which
  rewrites the entries only if some were installed. Someone who never ran
  `integrate` gets nothing written.
- Only files carrying the marker `one-mk: generated by` (or `OneMkGenerated` in
  a bundle) are replaced or removed, and only inside the entry folders and
  with a name starting `Send `; a user's own scripts there are left alone (tested).
  Symlinks are never ours, whatever they point at.
- Writing never overwrites: our own old entries are cleared first and each new
  one is created exclusively (no symlink is followed, nothing is truncated). If
  something that is not ours is at an entry's path (say your own Nautilus script
  called `Send to laptop`), that entry is **skipped**, the rest are written, the
  skipped paths are listed, `one-mk integrate` exits non-zero, and `install` and
  `pair`/`forget` only print a warning. Rename or move your file and run
  `one-mk integrate` again. The same goes for every target (Nautilus, Nemo,
  Dolphin, SendTo, Quick Action).
- A line break or NUL in the program's path or in `--home` is refused (with a
  message) before anything is written, by both `install` and `integrate`: it
  would end a line of the service file or of Dolphin's `Exec=` and start a new
  directive. Menu text values escape backslash, newline, carriage return and tab
  as the Desktop Entry specification says.
- Linux writes for a file manager only if its program is on `PATH` or its folder
  already exists.
- Failures: `--notify-errors` shows a desktop notification (D-Bus, falling
  back to `notify-send`, then stderr; osascript on macOS). Windows has no
  notification from one-mk yet, so its `.cmd` keeps the console open on
  an error.
- Nautilus passes names relative to the folder it shows; that is why the
  script does not rewrite them, and why `--` matters.

Tested here: file contents and quoting with hostile file and computer names;
the generated Nautilus script run by a real `sh` against a stub `one-mk` with
fifteen odd arguments (spaces, quotes, `$HOME`, backticks, newline, a leading
dash, `--`, `*`); write, refresh after a forget and removal with a user's lookalike
files present; a real `one-mk integrate` / `forget` / `integrate --remove` cycle
in a scratch `$HOME` and `$XDG_DATA_HOME`; the failure notification through a
stub `notify-send`; the Quick Action plists parse. **Not tested:** the entries
in a running Nautilus, Dolphin, Explorer or Finder (the `.workflow` format was
written from memory of what Automator saves; open one in Automator once to
confirm), and the Windows `.cmd` in Explorer.
