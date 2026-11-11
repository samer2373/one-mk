# Live wallpaper

oneMK can show one video or picture across all your screens, as one picture
that continues from screen to screen in the positions you gave them in
**Desk > Arrange**, and across your paired computers.

## Using it

Open the oneMK window, go to **Wallpaper**, and choose **Choose…** (or drop a
file on the page). Pick how it fills the screens (Fill, Fit or Stretch),
whether it is one picture **Across** all the screens (the default) or the
whole picture **Each** screen shows by itself, the same on every screen of
every computer, and which computers show it (**Play on**). **Pause** holds a video on its current
frame; **Turn off** stops it everywhere and puts your own wallpaper back.

From a terminal:

```
one-mk wallpaper set ~/Videos/waves.mp4     # or a .png / .jpg / .webp picture
one-mk wallpaper pause
one-mk wallpaper play
one-mk wallpaper status
one-mk wallpaper off
```

Every `wallpaper` command takes `--json` (`{"ok": true, "message": ...}`, or
`{"ok": false, "error": ...}` with a non-zero exit). `status` with no oneMK
running says the wallpaper is off because oneMK is not running; the other
commands say to start it. A wallpaper chosen while the screen is locked or the
computer asleep says it is paused for that reason and resumes on unlock. A file
that cannot be played is refused with the reason and is not kept.

Videos: MP4, MOV, M4V, WebM and MKV, played by the system's own decoder
(GStreamer on Linux). Pictures: PNG, JPEG and WebP everywhere, HEIC on macOS
and on Windows with the HEIF extension.

## What works where

| | Video | Picture | Status |
|---|---|---|---|
| Linux, X11 desktop | yes | GNOME-based desktops (GNOME, Ubuntu, Budgie, Unity, Pantheon) | **stable** |
| Linux, GNOME on Wayland | through oneMK's GNOME extension (GNOME 50) | yes | pictures **stable**, video **experimental** |
| Linux, layer-shell Wayland (KDE, Sway, Hyprland…) | yes | yes, drawn by oneMK | **experimental** (tested on Sway only) |
| Linux, other X11 desktops (KDE, Xfce, Cinnamon, MATE…) | yes | no: oneMK says so instead of pretending | video **stable** |
| macOS | yes | yes | **experimental**: not yet tested on a real Mac |
| Windows | yes | yes | **experimental**: not yet tested on a real PC |

Also experimental everywhere: one wallpaper **shared across several
computers** (kept in step over the network), and the **smaller copy** oneMK
makes of a very large video before sharing it (if that fails, the original
is used). The Wallpaper page and `one-mk wallpaper status` say which of these
apply to your computer.

## Your own wallpaper comes back

Before oneMK changes your desktop's wallpaper the first time, it saves your
own settings (the light and dark pictures and how they are placed) in its
settings folder, and **Turn off** puts exactly those back. A setting that was
at your desktop's default goes back to being the default. The saved copy is
kept on disk, so it survives a crash or a power cut: oneMK puts your wallpaper
back the next time it starts if it couldn't before.

`one-mk uninstall` also turns the wallpaper off, puts your own wallpaper back
and removes oneMK's GNOME extension.

## GNOME setup (video on Wayland)

GNOME on Wayland only lets an extension draw behind your windows, so video
needs oneMK's small GNOME extension. It is part of oneMK; nothing is
downloaded.

1. On the **Wallpaper** page choose **Install GNOME extension** (or run
   `one-mk wallpaper extension install`).
2. Log out and back in once. The extension turns itself on.
3. Choose a video.

`one-mk wallpaper extension status` shows where you are. The extension runs
on GNOME 50 (any 50.x release); on other GNOME versions oneMK says the
version is not supported, and pictures still work. Until the extension is
active, a video another computer shares with this one is shown as a still of
its first frame, and the Wallpaper page says so together with the step above.
`one-mk wallpaper extension uninstall` switches it off and removes it. Both
commands say what really happened and exit non-zero when a step failed: install
is refused on a GNOME version it does not run on (and on a computer without
GNOME), and one that copies the files but cannot switch the extension on says
so with the one step left; uninstall removes the files either way, and if
GNOME could not be told to switch it off it says `gnome-extensions disable
one-mk-wallpaper@one-mk.org` is the step left.

## Troubleshooting

- **"… does not exist" or "… is a folder"**: check the file name; choose a
  file, not a folder.
- **"This picture cannot be opened"**: the file is damaged or not a PNG, JPEG
  or WebP picture. Open it in an image viewer and save it as PNG.
- **"Picture wallpapers on Linux need GNOME …"**: your desktop keeps its
  wallpaper elsewhere. Set the picture in its own settings; videos still work.
- **"Install GStreamer to play video wallpapers"**: install your
  distribution's GStreamer "good" and "libav" plugin packages.
- **"The system could not decode this video"**: the video uses a format your
  system has no decoder for. Convert it to H.264 MP4 or VP9 WebM.
- **"Update oneMK on … to play video wallpapers"**: every computer that shows
  a shared wallpaper needs the same oneMK version.
- **The wallpaper player stopped**: choose Play to try again; `one-mk -v run`
  logs the reason.
- **Your own wallpaper did not come back**: run `one-mk wallpaper off` while
  oneMK runs. If it still doesn't, choose your wallpaper again in your
  desktop's settings.

## Privacy and safety

Videos and pictures go only to computers you paired, over oneMK's encrypted
connection, and are kept in oneMK's settings folder (`wallpaper-video`),
which is cleaned when you choose something else. Pictures are opened in a
separate helper process with size limits, so a damaged file can't take oneMK
down. Videos are decoded by your system's own media framework with your
user's permissions; only accept wallpapers from computers you trust.
