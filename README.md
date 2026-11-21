# Owl-Layoutd
> **Per-window keyboard layout memory for Hyprland... But like on KDE!**


# Featuring:


By default Hyprland applies one keyboard layout to all windows. If you type in more than one language, that gets annoying fast: English in the terminal, Russian in the messenger, and you keep switching by hand.

Owl-Layoutd remembers the layout **for each application** and restores it when you focus that window, just like KDE's "Application" switching policy. It is a tiny Rust daemon with zero dependencies: about 130 KB of private memory (≈2.2 MB RSS including shared libraries), and it sleeps when nothing happens.

I wrote it because `hyprland-per-window-layout` sometimes picked the wrong layout on my setup (four layouts and a lot of extra virtual "keyboards").

## How it works

- Listens to Hyprland events over its socket and sleeps when idle.
- Remembers the layout **per application** (window class), like KDE's "Application" switching policy. Every program keeps its own layout, even if you have several windows of it.
- Saves its memory to `~/.local/state/owl-layoutd/classes`, so layouts survive reboots.
- Treats overlays like Rofi as their own "application" (starts in the first layout until you change it).
- Tracks one real keyboard by name and ignores fake "keyboards" (power button, headphones, ydotool). Keeps all devices in sync.


## Configuration

- **Keyboard**: the daemon tracks `at-translated-set-2-keyboard` by default. Find yours
  with `hyprctl devices` and set it via `OWL_KEYBOARD=<name>` or the `KEYBOARD` constant in `src/main.rs`.
- **Overlays** (Rofi, launchers): add their layer-shell names to the `LAYERS` list in `src/main.rs`.
  Names are printed as `layer open: ...` in the debug log.
- **Reset memory**: delete `~/.local/state/owl-layoutd/classes`.
- **Debug**: run with `OWL_DEBUG=1 Owl-Layoutd` to print every decision. To keep the logs in the systemd journal:

```
exec-once = OWL_DEBUG=1 systemd-cat -t owl-layoutd Owl-Layoutd
```

Read them with `journalctl -t owl-layoutd -e`.


## Install

Requires Hyprland and Rust (`sudo apt install cargo` or your distro’s equivalent).

```bash
git clone https://github.com/OwlMyName/Owl-Layoutd.git
cd Owl-Layoutd
cargo build --release
cp target/release/Owl-Layoutd ~/.local/bin/
```

Add to `hyprland.conf`:

```
exec-once = Owl-Layoutd
```

Make sure `~/.local/bin` is in your `PATH`, or use the full path in `exec-once`.


## Tested on

Hyprland 0.56.0, layouts `us,ru,ua,cz`.


