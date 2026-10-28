<img src="assets/icon.svg" width="64" height="64" alt="">

# one-mk

Share one mouse and keyboard across your computers: smooth, fast, light,
and set up in under a minute.

> Status: early. Linux (Wayland/GNOME) works end to end; Windows and macOS
> backends are next.

## Quick start

On **both** computers:

```bash
one-mk pair        # both show the same 6-digit code; confirm on each
one-mk run         # start sharing
```

The computer you pair from first is placed to the right. To change it:

```bash
one-mk layout "other-pc" left
```

Other commands: `one-mk devices`, `one-mk forget <name>`, `one-mk id`,
`one-mk bench`.

**Emergency:** `Ctrl+Alt+Shift+Esc` always brings the pointer back home.

## Why it feels better

| | |
|---|---|
| **Smooth mouse** | Every mouse report is forwarded immediately at the mouse's native rate, with no batching to 60 Hz. |
| **Never stutters on packet loss** | Motion travels as QUIC datagrams carrying absolute positions. A lost packet is replaced by the next one, so nothing is retransmitted and nothing waits. |
| **Clicks land exactly where you aimed** | Each click carries the pointer position it was made at. |
| **No Wi-Fi hiccups** | While the pointer is remote, a tiny keepalive every 20 ms stops the Wi-Fi radio from entering power save, which otherwise causes 50–200 ms stalls. |
| **Same speed on every screen** | Acceleration is applied once, by the machine the mouse is plugged into. Positions are in logical pixels, so mixed DPI and scaling just work. |
| **No stuck keys** | Held keys are tracked and released when the pointer leaves, the peer disconnects, or traffic stops for 750 ms. |
| **Light** | Zero CPU when idle. The network side runs on a single thread, with one thread each for input capture and injection. |
| **Secure by default** | Mutual TLS 1.3 over QUIC. Devices are pinned by certificate fingerprint at pairing, using a commit-then-reveal numeric code (like Bluetooth) that defeats man-in-the-middle attacks. |
| **Easy** | No server/client roles, no IP addresses (mDNS discovery), no certificates to copy, no root and no udev rules on Linux. |

## Measure it

```bash
one-mk bench                          # loopback
one-mk bench --serve                  # on machine B
one-mk bench --connect 192.168.1.20:24901   # on machine A
```

## Layout

```
crates/proto   wire format + pure pointer/edge/key logic (no I/O, unit-tested)
crates/net     identity, QUIC, pairing, trusted devices, mDNS discovery
crates/input   platform input: Linux (XDG portals + libei); Windows/macOS next
crates/one-mk  the daemon and CLI
```

## Building

```bash
cargo build --release      # target/release/one-mk
cargo test
```

On Linux you need GNOME 45+ or KDE Plasma 6.1+ on Wayland. The first
`one-mk run` asks for permission once (input capture and remote control).

## Roadmap

1. ✅ Linux ↔ Linux mouse & keyboard, pairing, discovery, benchmark
2. Windows backend (Raw Input + SendInput, SYSTEM service for UAC/login screen)
3. macOS backend (CGEventTap + CGEventPost, permission guide)
4. Clipboard (lazy, text/images/files) and drag-and-drop file transfer
5. Tray app with drag-and-drop layout editor; "push toward it" layout learning
6. Gaming mode (relative motion), connection doctor, auto-update
