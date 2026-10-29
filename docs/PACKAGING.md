# Packaging

How one-mk is packaged for each system, how to build each package, and what
has and has not been tried on real systems.

| Format | Script | Output | Built where | Status |
|---|---|---|---|---|
| plain `.tar.gz` / `.zip` | the "Package" step of `.github/workflows/release.yml`, or `scripts/package-tarball.sh` | `one-mk-<target>.tar.gz` / `.zip` | CI, any | in use since the first release; what `install.sh` downloads |
| Debian `.deb` | `scripts/package-deb.sh` | `one-mk_<version>-1_amd64.deb` | Linux with `dpkg-deb` | **built and checked here** |
| AppImage | `scripts/package-appimage.sh` | `one-mk-<version>-x86_64.AppImage` | Linux with `appimagetool` | AppDir built and run here; the `.AppImage` file itself **not built here** |
| Flatpak | `scripts/package-flatpak.sh` + `packaging/flatpak/` | manifest (and a bundle with `flatpak-builder`) | Linux with `flatpak-builder` | **written, untested** |
| Windows installer | `scripts/package-windows.sh` + `packaging/windows/one-mk.iss` | `one-mk-<version>-x64-setup.exe` | Windows with Inno Setup | **written, untested** |
| macOS `.app` / `.dmg` | `scripts/package-macos.sh` + `packaging/macos/` | `one-mk-<version>-arm64.dmg`, `-x64.dmg` | macOS | **written, untested** (the `.app` layout is built on Linux with `APP_ONLY=1`) |
| Homebrew cask and formula | `scripts/package-homebrew.sh` + `packaging/homebrew/` | `one-mk.rb`, `one-mk-source.rb` | anywhere | **written, untested** |

Everything lands in `dist/` (or `$OUT_DIR`). The scripts never build the
programs themselves, so CI and a developer package exactly the same
binaries:

```sh
cargo build --release --locked -p one-mk -p one-mk-tray -p one-mk-app
sh scripts/package-deb.sh
sh scripts/check-packaging.sh        # builds and checks the Linux packages in /tmp
```

With `TARGET=<triple>` the scripts look in `target/<triple>/release`
(the release workflow builds with `--target`). On a machine short of memory
the release profile's fat LTO can run out of it; for a local check build with
`CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`
(the packages are then bigger and a little slower, but otherwise the same).

## One version

The version is `[workspace.package] version` in the root `Cargo.toml`; every
crate says `version.workspace = true`. Nothing else holds a version:

- `sh scripts/version.sh` prints it; every package script uses it.
- The release workflow starts with `scripts/version.sh --check $GITHUB_REF_NAME`,
  so tagging `v0.2.0` while `Cargo.toml` says `0.1.0` stops before anything is
  built.
- To release: change the version, commit, tag `vX.Y.Z`, push the tag.
  A pre-release version such as `0.2.0-rc1` becomes `0.2.0~rc1-1` in the
  `.deb` so that it sorts before `0.2.0`.
- `one-mk --version` prints `one-mk <version>` (clap reads it from the crate).
  `one-mk-app` and `one-mk-tray` have no `--version`; they print their usage
  for an unknown option.

## Shared files

The systemd user unit and the desktop entries are not copied into the
packaging folder: `one_mk_ctl::install::packaged_files` produces them from the
same functions `one-mk install` uses to write them into the home folder, and
`scripts/package-common.sh` runs them through the `packaging-files` example
(`cargo run -p one-mk-ctl --example packaging-files -- ROOT /usr/bin`). A
unit test checks that the packaged unit is the installed unit with another
program path. Hand-written, static files are in `packaging/linux/`:
the window's `.desktop`, the AppStream metainfo, the man page, the copyright
file and the maintainer scripts. Icons come from
`docs/brand/app-icon/` (hicolor PNGs and SVG for Linux, `.icns` for macOS,
`.ico` for the Windows installer).

## Debian `.deb`

`scripts/package-deb.sh` uses `dpkg-deb` directly (no debhelper, no
cargo-deb). The package contains:

| Path | What |
|---|---|
| `/usr/bin/one-mk`, `one-mk-tray`, `one-mk-app` | the programs |
| `/usr/lib/systemd/user/one-mk.service` | the user service (`systemctl --user enable --now one-mk`) |
| `/usr/share/applications/one-mk.desktop` | the window's launcher |
| `/usr/share/applications/io.github.onemk.OneMk.desktop` | hidden entry that gives the daemon its app id for the GlobalShortcuts portal |
| `/etc/xdg/autostart/one-mk-tray.desktop` | starts the tray icon at login (a conffile; see below) |
| `/usr/share/icons/hicolor/*/apps/one-mk.*` | icons, 16 px to scalable |
| `/usr/share/metainfo/io.github.onemk.OneMk.metainfo.xml` | AppStream |
| `/usr/share/man/man1/one-mk.1.gz` (+ symlinks for the tray and the app) | man page |
| `/usr/share/doc/one-mk/{copyright,changelog.Debian.gz}` | licence (MIT or Apache-2.0) and changelog |

Design choices:

- **Installing starts nothing, enables nothing.** The maintainer scripts
  (`packaging/linux/postinst`, `postrm`) only refresh the icon and desktop
  caches; there is no `systemctl`, `deb-systemd-helper` or `adduser` in them,
  and nothing runs as root that a user's login depends on. Starting at login
  stays a per-user decision: `one-mk install`, or
  `systemctl --user enable --now one-mk`. `scripts/check-packaging.sh`
  fails if a maintainer script mentions `systemctl`, `start` or `enable`.
- **The tray autostarts, the daemon does not.** The tray only shows the state
  of the service the user turned on, and offers to turn it on; it does not
  capture input. To stop it, copy the file to `~/.config/autostart/` with
  `Hidden=true`, or use the desktop's startup applications.
  `one-mk install` writes `~/.config/autostart/one-mk-tray.desktop`, which
  overrides the packaged one by name, so it never starts twice. The same goes
  for the unit: `~/.config/systemd/user/one-mk.service` wins over the packaged one.
- **Depends** comes from `dpkg-shlibdeps` when it is installed (libc6 and
  libgcc-s1 today). The window opens OpenGL, xkbcommon and Wayland at run time,
  so those are `Recommends`, which keeps the command-line part installable on a
  server. Build on the oldest distribution you support (the release workflow
  uses ubuntu-22.04, glibc 2.35), because the glibc dependency is the version
  of the build machine's.
- **Reproducible.** File times and modes are normalised and
  `SOURCE_DATE_EPOCH` is the commit's time; two builds give the same bytes
  (`check-packaging.sh` compares the checksums). The builder's time zone does
  not matter: `touch -t` reads its stamp in local time, so it runs with
  `TZ=UTC0` (`SOURCE_DATE_EPOCH=1700000000` gave mtime 1699989200 under
  `Asia/Amman` before), and `check-packaging.sh` builds the `.deb` under two
  other zones and compares checksums.

Verified here (Debian-family host, release build with LTO off): contents with
`dpkg-deb -c`, fields with `dpkg-deb -I`, extraction with `dpkg-deb -x` into a
throwaway root and `usr/bin/one-mk --version` run from there,
`desktop-file-validate` (clean on the three entries), `appstreamcli validate
--no-net` (clean), `systemd-analyze --user verify` on the unit (clean, and it
does flag a deliberately broken copy), no world-writable or setuid files.
**Not run:** `lintian` (not installed here), an `apt install` on a real
system, and an upgrade or removal. Known lintian-style points worth checking
when it is available: the changelog is a one-line pointer to the GitHub
release notes, and there is no separate upstream changelog.

## AppImage

`scripts/package-appimage.sh` lays out `dist/one-mk.AppDir`, runs
`AppRun --version` to prove the layout works, and then calls `appimagetool`
if it can find one (`APPIMAGETOOL=/path`, or on `PATH`).

- `AppRun` (`packaging/linux/AppRun`): with no arguments it starts the window;
  `app` and `tray` start the window and the tray explicitly; anything else goes
  to `one-mk`, so `./one-mk-0.1.0-x86_64.AppImage pair` works.
- Inside an AppImage the running copy is in a temporary mount. When one-mk is
  started from one, `one-mk install` and `one-mk integrate` therefore write the
  path of the `.AppImage` file (from `$APPIMAGE`), not the mount, into the
  login service and the file manager entries, and no tray entry (the tray is
  not next to that path). Keep the file where it is after installing.
- The release workflow downloads appimagetool 1.9.0 and checks it against
  a pinned SHA-256 (`46fdd785...`). That hash is of the file as downloaded
  while writing this, not independently verified; compare it with the
  project's release page before trusting it.
- The `.desktop` file inside names `one-mk-app`; AppImageLauncher and similar
  tools rewrite the `Exec` line to the file.
- Not done: an update mechanism (zsync), and an AppImage for ARM (the script
  builds one if run on aarch64, untried).

## Flatpak (untested)

`packaging/flatpak/io.github.onemk.OneMk.yml.in` is a manifest that repackages
the release tarball (a `simple` module), so the Rust build does not have to
work offline inside the sandbox. `scripts/package-flatpak.sh [URL_OR_FILE]`
fills it in; with `flatpak-builder` installed it also builds a bundle. Nothing
in it has been run: `flatpak-builder` is not available here. Runtime
`org.freedesktop.Platform//25.08` is the newest at the time of writing;
adjust it to what exists.

**What is published is a source bundle, not the bare manifest.** The script
copies the desktop file, the metainfo and the two icons the manifest installs
into `sources/` next to it (each with its `sha256:`), writes relative `path:`s
only, and packs the folder as `one-mk-<version>-flatpak-sources.tar.gz`
(sorted, clamped times, so the same bytes every time). Given a URL, the
tarball source is that URL with its checksum (from a download made by the
script); given a file, the tarball is copied into the bundle too. The release
workflow uploads the bundle; extract it anywhere and run `flatpak-builder` in
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
  with the GlobalShortcuts portal, which is why Flatpak and the `.deb`'s hidden
  desktop entry agree.

## Windows installer (untested)

`packaging/windows/one-mk.iss` is an Inno Setup script, chosen over WiX
because it needs no extra extension for a firewall rule or a custom action
and the whole thing is one readable file. `scripts/package-windows.sh`
(Git Bash, with `iscc` installed; CI installs it with Chocolatey) builds
`one-mk-<version>-x64-setup.exe`. It:

- installs the three programs to `C:\Program Files\one-mk` (it needs an
  administrator for that and for the firewall; the per-user part below runs as
  the person who started Setup);
- adds a Start menu shortcut to the window;
- adds two inbound firewall rules for `one-mk.exe`, **private and domain
  networks only**: UDP for the program (its link on port 24900, and the
  pairing port, which is picked at random so it cannot be named), and mDNS on
  UDP 5353. A network Windows calls *Public* stays closed; `one-mk doctor` says
  what to do there;
- optionally (a checkbox, on by default) runs `one-mk.exe install` as the
  original user, which writes the per-user `Run` entry (the tray starts the
  daemon hidden) and the *Send to* entries;
- optionally adds the folder to the machine's `PATH`, as one whole entry and
  only if it is not there already; when it adds it, it records that in
  `HKLM\SOFTWARE\one-mk` (`PathEntry`);
- on uninstall runs `one-mk.exe uninstall` (removes its own `Run` entry and the
  SendTo entries of the account running the uninstaller), stops any running
  one-mk, deletes both firewall rules and takes the folder out of `PATH`, **only
  if the install recorded that it put it there, and only the entry equal to the
  install folder (ignoring case)**. A neighbour such as
  `C:\Program Files\one-mk-tools` is never cut (the old code searched for the
  folder as a prefix and turned that entry into `C:\Windows-tools`-style
  garbage). The identity and paired computers in `%LOCALAPPDATA%\one-mk` are
  left, deliberately;
- on uninstall also removes the start-at-sign-in value `one-mk` from every
  *loaded* user registry (`HKEY_USERS\<SID>\Software\Microsoft\Windows\CurrentVersion\Run`)
  whose data starts a program inside the install folder. This matters when a
  standard user installs with another administrator's credentials: Setup writes
  the entry as the original user, but the elevated uninstaller's own
  `HKEY_CURRENT_USER` is the administrator's. The SIDs come from the machine's
  `ProfileList`; for a profile whose registry is not loaded (that user is signed
  out) the uninstall log (`/LOG`) names the SID and says to delete the value by
  hand or run `one-mk uninstall` as them. (Instead of recording the user's SID
  at install, which would need code run as the original user and a way to hand
  the answer to the elevated Setup, the uninstaller matches the value's data
  against its own folder: no value that points somewhere else is touched.)

The CLI also checks ownership before removing the current account's `Run\\one-mk`
value: its executable must be in the running executable's folder. Quoted paths,
case differences, slash direction, and trailing folder separators are handled;
an unrelated portable install is preserved. Windows 8.3 short-name aliases are
not resolved by this comparison. Windows registry behavior is review-only here;
the ownership comparison has platform-independent unit coverage.

The PATH and startup-value logic is `packaging/windows/path-entries.pas`,
included by the `.iss`; `scripts/check-windows-path.sh` (run by
`check-packaging.sh`) runs the same steps in awk on 26 cases, among them the
corrupting PATH above. **That is all that was run:** the Pascal itself, the
registry access and the whole installer are written on Linux and have never
run on Windows (the Pascal is a statement-for-statement twin of the awk, not
the thing tested).

Things to check on a real machine: that `runasoriginaluser` really lands the
`Run` entry in the signed-in user's registry when Setup is started by
"Run as administrator" from another account; the firewall rules
(`netsh advfirewall firewall show rule name=all | findstr one-mk`); that
`CloseApplications=force` stops a running tray during an upgrade; that the
installer's `AppId` stays the same across releases. The installer is not
code-signed, so SmartScreen will warn.

## macOS `.app` and `.dmg` (untested)

`scripts/package-macos.sh` builds `one-mk.app` and wraps it in a `.dmg` (with an
`Applications` link) using `hdiutil`. The bundle:

```
one-mk.app/Contents/Info.plist        com.one-mk.app, version from Cargo.toml
              MacOS/one-mk-app        the window (CFBundleExecutable)
              MacOS/one-mk            the command line and the service
              MacOS/one-mk-tray       the menu bar icon
              Resources/onemk.icns    from docs/brand/app-icon/macos
```

The three programs sit side by side, so `one-mk install` (which looks for
`one-mk-tray` next to itself) registers LaunchAgents that point inside the
app, and the permission grants macOS keeps per program (Accessibility, Input
Monitoring) stay put as long as the app does. The cask links
`one-mk` onto `PATH`.

- Signing is ad hoc (`codesign --sign -`) unless `MACOS_SIGN_IDENTITY` names a
  Developer ID; there is **no notarisation**, so the first launch needs
  right-click, Open, or `xattr -dr com.apple.quarantine one-mk.app`. The cask
  says so in its caveats.
- There is one `.dmg` per architecture (the runners build natively) rather than
  a universal binary. Joining them with `lipo` is a later step.
- `packaging/homebrew/one-mk.rb.in` is the cask (the normal way to install by
  Homebrew), `one-mk-source.rb.in` a formula that builds from source.
  `scripts/package-homebrew.sh` fills in checksums; the release workflow attaches
  both files to the release, and putting them in a tap
  (`samer2373/homebrew-tap`) is by hand.
- The workflow's Intel runner is `macos-15-intel`; the old `macos-13` image was
  retired. If GitHub renames it again, change it in the matrix.

## The release workflow

On a `v*` tag, `release.yml`:

1. checks the tag against `Cargo.toml`;
2. per OS, builds the three programs, makes the plain archive as before, and
   then the platform packages: `.deb` and AppImage on ubuntu-22.04, `.dmg` on
   the two macOS runners, the installer on Windows. They are uploaded as
   `packages-<target>` artifacts;
3. `publish` collects everything into one release with a `SHA256SUMS`, then
   adds the Homebrew cask and formula and the Flatpak source bundle (they need
   the release's own URLs and checksums).

None of the new steps has run in CI yet (the workflow was edited without being
able to run it); the first tag is the test. Packages are unsigned throughout.

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
