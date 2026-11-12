# Security model

oneMK sees every key you type and moves the pointer on your other computers, so it is
built to talk only to computers you have paired, to keep its secrets private to your account,
and to do nothing with your input that you did not ask for. This page says what it relies on and
where the limits are. To report a vulnerability, see [SECURITY.md](../SECURITY.md).

## Between computers

- **Mutual TLS 1.3 over QUIC.** Each computer has its own certificate and key, created on first
  use (`one-mk id` shows the name and fingerprint). Both sides check the other's certificate
  fingerprint on every connection.
- **Pairing pins the fingerprint** with a commit-then-reveal numeric code (like Bluetooth)
  that defeats a man in the middle: the two computers show the same six digits, and you
  confirm on both. Always compare the code on both screens; a familiar computer name is not
  enough. Only paired computers can connect; others are rejected.
- **Invitations** (`one-mk invite` / `one-mk join`) pair without sitting at both computers:
  a one-time, expiring link or QR code. Anyone holding the link can join if they use it first,
  so treat it like a password; with the default confirmation, the host also checks the joining
  computer's fingerprint and the comparison code. Skipping both prompts
  (`join --yes`, `invite --no-confirm`) removes human verification. Details, including what
  appears in process lists and journals: [PAIRING-INVITE.md](PAIRING-INVITE.md).
- **Forgetting a computer** (`one-mk forget`) revokes it. When the other computer next
  connects it is refused, says so (`status`, `doctor` and the window show "removed this
  computer", with what to do) and only retries every 10 minutes, until it is forgotten there
  too or paired again.
- **Discovery** uses mDNS on the local network to find paired computers; it does not
  authenticate anything. Without it, use `--connect <ip>:24900`.
- **Network exposure.** One UDP port (default 24900) and, for discovery, mDNS (UDP 5353).
  There is no server role and no account.

## On each computer

- **Settings and keys** live in your user's config folder (`~/.config/one-mk` on Linux), mode
  0700 for the folder and 0600 for the private key and token files. The control channel for
  `status`, `layout` and the window is a Unix socket with mode 0600 on Linux and macOS; on
  Windows it is a localhost port protected by a random token in a file in your private local
  settings folder.
- **Received files** cannot escape the download folder or overwrite anything, and every file is
  checked with BLAKE3. Limits on counts and sizes are in [FILES.md](FILES.md).
- **Clipboard.** Copies a password manager marked as sensitive are not sent, not handed out
  and not remembered. Fetching your clipboard from another computer is off until you allow it.
  History is off by default and, when you turn on persistence, is plain text on disk
  ([CLIPBOARD.md](CLIPBOARD.md)).
- **Hooks and `run:` key actions** run programs as you from `settings.toml`. The daemon refuses
  them when the settings file or its folder is writable by other users or owned by someone
  else, and offers a deliberate, separately stored override. On Windows the settings file's
  permissions are not checked; keep the folder private ([AUTOMATION.md](AUTOMATION.md)).
- **Emergency chord.** `Ctrl+Alt+Shift+Esc` always brings the pointer home and cannot be remapped.
  Everything oneMK holds down is released whenever the pointer leaves, a computer disconnects,
  traffic stops for 750 ms, or control is taken back.
- **Local key rules** (Windows and macOS only, experimental, off by default) intercept your own
  keyboard; they are labelled as untested on real hardware, and the chord above turns them off.
- **No root.** oneMK runs as you. Linux needs no udev rules or elevated rights; the only
  elevated step anywhere is the optional `.deb` update, below.

## Updates and installers

Releases carry an Ed25519-signed list of every file with its size and SHA-256, and the public
key is built into oneMK. The in-app updater, the `.deb` root step, `install.sh` and the macOS
app check are explained in [UPDATES.md](UPDATES.md) and, in terms of what the signature does
and does not protect, in [SECURITY.md](../SECURITY.md). A daily update check contacts
`api.github.com` once at startup; switch it off with `one-mk update --check-on-start off` (or in
Settings). The Windows installer is not Authenticode-signed yet, so SmartScreen warns about it.

## Known limits

- **X11 is not a protection boundary.** Any program that can talk to your X server can read keys
  and fake input, and experimental edge drops can be faked the same way; turn them off in an
  untrusted X session ([DRAG-FILES.md](DRAG-FILES.md)).
- **A compromised account is out of scope.** Someone who can run programs as you can read your
  settings and keys anyway.
- **Locked screens.** A locked computer never forwards its keyboard and mouse to another one (no
  edge crossing, `switch` or `broadcast`; a visit in progress ends when it locks) and refuses to be
  controlled, so a password typed at a lock screen stays on that computer and nothing is typed into
  one from another ([EDGE-PUSH.md](EDGE-PUSH.md#a-locked-screen)).
- **Windows secure desktop.** oneMK does not send input to the Windows lock screen, sign-in screen
  or UAC prompts, and there is no Windows service. Why that is deliberate: [WINDOWS.md](WINDOWS.md).
- **Wallpaper videos** are decoded by your system's media framework with your rights, so accept
  wallpapers only from computers you trust ([LIVE-WALLPAPER.md](LIVE-WALLPAPER.md)).
- **Not independently audited.** The code has had no outside security review, and the Windows
  and macOS backends have not run on real hardware ([README](../README.md#platform-support)).
