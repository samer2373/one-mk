# Why it feels good, and how to measure it

- **Smooth mouse.** Every mouse report is forwarded immediately at the mouse's native rate,
  with no batching to 60 Hz.
- **No stutter on packet loss.** Motion travels as QUIC datagrams carrying absolute positions; a
  lost packet is replaced by the next one, so nothing is retransmitted and nothing waits.
- **Clicks land where you aimed.** Each click carries the pointer position it was made at.
- **Idle position repeats.** While the pointer is on another computer and not moving, its
  position is repeated every 20 ms. This tells the other side you are still there and tends to
  keep Wi-Fi radios from dozing (which can add noticeable stalls). It is a few bytes, and only
  while the pointer is remote.
- **Same speed on every screen.** Acceleration is applied once, by the computer the mouse is
  plugged into; positions are logical pixels, so mixed DPI and scaling work.
- **No stuck keys.** Held keys are tracked and released when the pointer leaves, a computer
  disconnects, traffic stops for 750 ms, or control is taken back. However busy the network,
  these deadlines are checked first.
- **Light.** Measured idle CPU of the service is approximately zero with no peers. Each connected
  peer costs approximately 4-5 timer wakeups per second for keep-alive. This is measured
  behaviour, not a zero-work guarantee; local commands also wake it. Networking runs on one
  thread, with one thread each for input capture and injection. The service uses about 20 MB of
  memory. The optional window is heavier because it draws a 3D scene with the graphics card:
  on a normal desktop it idles at a fraction of a percent of one core, but it is about 125 MB of
  memory, which is why closing it (it is not needed for sharing) frees all of that. How the
  wake-ups were measured: [IDLE-WAKEUPS.md](IDLE-WAKEUPS.md).
- **Easy.** No server/client roles, no IP addresses (mDNS discovery), no certificates to copy,
  no root and no udev rules on Linux.

## Measure it yourself

```bash
one-mk bench                                # loopback
one-mk bench --serve                        # on computer B
one-mk bench --connect 192.168.1.20:24901   # on computer A (use the address and port `--serve` prints)
```

`one-mk bench --peer NAME` measures a paired computer that is running oneMK. `one-mk status`
shows `rtt 4 ms` for each connected computer, and the window draws it live. Under about 40 ms is
smooth.
