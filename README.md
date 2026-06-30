# Radial Launcher

A fast, lightweight, and highly customizable native radial application launcher designed for wlr-based Wayland compositors (such as Sway, River, and Wayfire). Built with **Rust**, **GTK4 (gtk-rs)**, **cairo-rs**, and **wlr-layer-shell**.

![Design Preview](https://raw.githubusercontent.com/pentamassiv/gtk4-layer-shell-gir/main/logo.png) *Gtk4 Layer Shell powered launcher*

---

## Features

*   **Native & Fast** — Built directly on Wayland protocols (wlr-layer-shell) and rendered in GTK4 via Hardware Acceleration.
*   **Zero-Daemon Design** — Runs only when toggled and exits immediately on selection or dismissal. An IPC socket handles toggle signaling.
*   **Config-as-Code** — A simple TOML format defines the circular hierarchy.
*   **Runtime CSS Themes** — Hot-swappable stylesheets without recompilation or restart.
*   **Keyboard + Scroll Navigation** — Full support for mouse-less operation using Tab/Arrow keys or the mouse scroll wheel.
*   **Submenu Traversal** — Seamless folder structures, with automatic circular "Back" slices.
*   **Built-in GUI Editor** — Launch with `settings` to manage menu items, icons, and themes graphically.

---

## Interaction & Navigation

| Input Event | Action |
|---|---|
| **Escape** | Dismiss / Close |
| **Right-Click** or **Click Outside** | Dismiss / Close |
| **Tab** / **Down Arrow** / **Right Arrow** | Cycle forward through wedges |
| **Shift+Tab** / **Up Arrow** / **Left Arrow** | Cycle backward through wedges |
| **Scroll Wheel (Up/Down)** | Cycle backward/forward through wedges |
| **Return** / **Space** / **Left-Click** | Execute action or enter submenu |
| **Click Center Circle Hub** | Go back to parent menu |

---

## Installation & Compilation

### Dependencies

Ensure you have **GTK4** and **gtk4-layer-shell** installed on your system.

On Arch Linux:
```bash
sudo pacman -S gtk4 gtk4-layer-shell
```

### Build

```bash
cargo build --release
```

The compiled binary will be located at `target/release/radial-launcher`.

---

## Usage & Startup Modes

```bash
# Open the launcher menu (Default)
radial-launcher open

# Open the GUI settings editor
radial-launcher settings

# Hot-reload themes and menu config on a running instance
radial-launcher reload
```

---

## Compositor Setup

Bind the launcher to a hotkey in your compositor config:

### 1. Sway (`~/.config/sway/config`)

```sway
# Toggle launcher
bindsym $mod+Space exec /path/to/radial-launcher open
```

### 2. River (`~/.config/river/init`)

```bash
# Toggle launcher
riverctl map normal Super Space spawn "/path/to/radial-launcher open"
```

### 3. Wayfire (`~/.config/wayfire.ini`)

```ini
[command]
binding_launcher = <super> KEY_SPACE
command_launcher = /path/to/radial-launcher open
```

---

## Customization

Config files are automatically generated on first launch under:
*   Menu Layout: `~/.config/radial-launcher/menu.toml`
*   UI Configuration: `~/.config/radial-launcher/config.toml`
*   Custom Themes: `~/.config/radial-launcher/themes/`

### Example `menu.toml`

```toml
[[menu]]
label = "Apps"
icon = "folder"

  [[menu.children]]
  label = "Terminal"
  icon = "utilities-terminal"
  action = { type = "exec", cmd = "alacritty" }

  [[menu.children]]
  label = "Browser"
  icon = "firefox"
  action = { type = "exec", cmd = "firefox" }

[[menu]]
label = "System"
icon = "system-shutdown"

  [[menu.children]]
  label = "Reboot"
  icon = "system-reboot"
  action = { type = "shell", cmd = "systemctl reboot" }
```

### Example `config.toml`

```toml
[ui]
theme = "gruvbox"   # Loads themes/gruvbox.css
font = "Sans 11"
```
