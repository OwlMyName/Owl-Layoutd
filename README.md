# Owl-Layoutd
> **Per-window keyboard layout memory for Hyprland... But like on KDE!**


# Featuring:


Remembers window layouts in Hyprland. By default, Hyprland applies a single keyboard layout globally across all windows. However, if you use more than one language, this becomes a hassle. While hyprland-per-window-layout exists, in my case it sometimes picked the wrong layout. This tiny daemon should solve the problem completely! With it, managing layouts feels just like in KDE, Windows, or any other sane OS. Uses about 130 KB of private memory (≈2.2 MB RSS including shared libraries). Simple, reliable and efficient.

## How it works

- Listens to Hyprland events over its socket, sleeps when idle.
- Remembers the layout of each window and restores it on focus.
- Ignores fake "keyboards" (power button, headphones, ydotool).
- Each new window can start with its own layout, based on its class.

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

## Configuration

Start layouts for new windows are set in the `RULES` table
at the top of `src/main.rs` (0 = first layout, 1 = second, ...).
Find a window class with `hyprctl clients | grep class`.
Rebuild after editing.

## Tested on

Hyprland 0.56.0, layouts `us,ru,ua,cz`.


