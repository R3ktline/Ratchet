<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/ratchet-horizontal-color-on-dark.svg">
    <img alt="Ratchet" src="assets/ratchet-horizontal-color.svg" width="360">
  </picture>
</p>

<p align="center">
  <b>Handwritten notes, simple and fast.</b><br>
  A pen-first notes app for drawing tablets and 2-in-1s.
</p>

<p align="center">
  <a href="https://github.com/R3ktline/Ratchet/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/R3ktline/Ratchet?color=d8442c"></a>
  <img alt="Linux and Windows" src="https://img.shields.io/badge/platform-Linux%20%7C%20Windows-4a86e8">
  <img alt="Built with Rust and GTK 4" src="https://img.shields.io/badge/built%20with-Rust%20%2B%20GTK%204-dea584">
  <a href="LICENSE"><img alt="License: CC BY-NC-SA 4.0" src="https://img.shields.io/badge/license-CC%20BY--NC--SA%204.0-lightgrey"></a>
</p>

<p align="center">
  <a href="#-install">Install</a> ·
  <a href="#-features">Features</a> ·
  <a href="#%EF%B8%8F-shortcuts">Shortcuts</a> ·
  <a href="#%EF%B8%8F-build-from-source">Build</a> ·
  <a href="#-license">License</a>
</p>

https://github.com/user-attachments/assets/540403c2-ee9a-4a13-bedb-fd8785635ce8

## ✨ Features

- 📚 **Notebooks → sections → pages** in a sidebar, with drag and drop, inline
  rename (double-click or F2), colors, and a trash you can undo from
- 🖊️ **Pressure-sensitive ink** with stroke smoothing; pens, highlighters and
  an eraser that either deletes whole strokes or cuts them
- 📐 **Shape snapping**: hold the pen still at the end of a stroke to turn it
  into a straight line (15° steps), an arrow or a circle
- 🎯 **Radial menu** that opens from a pen side button or by holding the pen still
- ➰ **Lasso selection**: move, resize, restyle, copy/cut/paste and duplicate
- 🔤 **Text boxes** with bold, italic, underline, bullets and inline LaTeX math
  (`$\int_0^1 x^2\,dx$`)
- 📄 **Page layouts**: infinite canvas or stacked sheets (A3–A5, Letter, Legal,
  Tabloid, slides, custom sizes), with blank, lined, grid or dotted backgrounds
- 🧰 **Minimal toolbar**: one pen, eraser, select and pan to start; add more
  pens, highlighters and the text tool from the "+" menu
- 📤 **Export** pages to PDF or PNG
- 🌗 Follows the system's light or dark theme (light if it has none), or pick
  one yourself

### 📁 Your notes are just files

Notes are plain folders and JSON files, so you can sync them with any tool
(Syncthing, Nextcloud, Git…):

| Platform | Default location |
|----------|------------------|
| 🐧 Linux    | `~/Documents/Ratchet` |
| 🪟 Windows  | `%USERPROFILE%\Documents\Ratchet` |

You can pick a different folder from the main menu. App settings are stored
in `~/.config/ratchet/settings.json` on Linux and
`%LOCALAPPDATA%\ratchet\settings.json` on Windows.

## 📦 Install

Download the latest version from the
[releases page](https://github.com/R3ktline/Ratchet/releases/latest).

**🐧 Linux:** get `Ratchet-<version>-x86_64.AppImage`, make it executable and
run it:

```sh
chmod +x Ratchet-*-x86_64.AppImage
./Ratchet-*-x86_64.AppImage
```

It runs on distros from about 2025 on (glibc 2.42 or newer: Fedora 43+,
Ubuntu 25.10+, Arch). On older distros, [build from source](#%EF%B8%8F-build-from-source)
on a newer distro release or in a toolbox.

**🪟 Windows 10/11 (64-bit):** run `Ratchet-<version>-x64-setup.exe`. It
installs for your user without admin rights, or for all users if you
choose so on the first page. The installer is not code-signed yet, so
SmartScreen may warn: click *More info → Run anyway*.

## ⌨️ Shortcuts

| Action | Shortcut |
|--------|----------|
| New page | Ctrl+N |
| New notebook | Ctrl+Shift+N |
| Rename page | F2 |
| Export page | Ctrl+E |
| Undo / Redo | Ctrl+Z / Ctrl+Shift+Z (or Ctrl+Y) |
| Zoom in / out / reset | Ctrl++ / Ctrl+− / Ctrl+0 |
| Toggle sidebar | F9 |
| Fullscreen | F11 (Esc to leave) |
| Preferences | Ctrl+, |
| Copy / Cut / Paste / Duplicate selection | Ctrl+C / Ctrl+X / Ctrl+V / Ctrl+D |
| Select all | Ctrl+A |
| Text tool | T |
| Delete selection | Delete or Backspace |
| Bold / Italic / Underline (in a text box) | Ctrl+B / Ctrl+I / Ctrl+U |

### 💡 Pen tips

- **Snap to a shape:** finish a stroke and hold the pen still to snap it to a
  line, arrow or circle. Keep holding and move to adjust it.
- **Radial menu:** map a pen side button to it in *Preferences → Pen*, or hold
  the pen still on the page. Release the button or lift the tip over an item
  to pick it.
- **Math in text:** wrap LaTeX in `$…$` for inline math or `$$…$$` for display
  math.

## 🛠️ Build from source

You need **Rust 1.85+** (2024 edition), **GTK ≥ 4.18** and
**libadwaita ≥ 1.8** with their development headers, a C toolchain and
`pkg-config`. Check what you have with:

```sh
pkg-config --modversion gtk4 libadwaita-1
```

<details>
<summary><b>🐧 Linux</b></summary>

<br>

If your distro's GTK or libadwaita is too old (for example Ubuntu 24.04 or
Debian 13), the build will fail. Use a newer distro release or build inside
a Fedora/Arch toolbox or distrobox.

**1. Install dependencies**

Arch / CachyOS / Manjaro:

```sh
sudo pacman -S --needed base-devel rust gtk4 libadwaita
```

Fedora (42+):

```sh
sudo dnf install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Ubuntu (25.10+) / Debian testing:

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**2. Build and run**

```sh
git clone https://github.com/R3ktline/Ratchet.git
cd Ratchet
cargo run --release
```

The binary ends up at `target/release/ratchet`.

**3. Install (optional)**

To put `ratchet` on your `PATH` and add it to your app launcher with its icon:

```sh
cargo install --path .
install -Dm644 data/app.ratchet.notes.desktop ~/.local/share/applications/app.ratchet.notes.desktop
install -Dm644 data/icons/scalable/apps/app.ratchet.notes.svg ~/.local/share/icons/hicolor/scalable/apps/app.ratchet.notes.svg
```

This assumes `~/.cargo/bin` is on your `PATH`, where `cargo install` puts the binary.

</details>

<details>
<summary><b>🪟 Windows</b></summary>

<br>

On Windows, GTK is most easily installed through [MSYS2](https://www.msys2.org/).
These steps build Ratchet with MSYS2's own Rust toolchain in the **UCRT64**
environment.

**1. Install MSYS2** from <https://www.msys2.org/>, keeping the default
location `C:\msys64`.

**2. Install dependencies.** From the Start menu, open **MSYS2 UCRT64** (not
"MSYS2 MSYS"), then run:

```sh
pacman -Syu
```

If it asks you to close the window, do so, reopen **MSYS2 UCRT64**, and run
`pacman -Syu` once more. Then install the toolchain and libraries:

```sh
pacman -S --needed git \
  mingw-w64-ucrt-x86_64-rust \
  mingw-w64-ucrt-x86_64-gcc \
  mingw-w64-ucrt-x86_64-pkgconf \
  mingw-w64-ucrt-x86_64-gtk4 \
  mingw-w64-ucrt-x86_64-libadwaita
```

**3. Build and run.** Still in the **MSYS2 UCRT64** shell:

```sh
git clone https://github.com/R3ktline/Ratchet.git
cd Ratchet
cargo run --release
```

The first build takes a few minutes. The Ratchet icon is embedded in
`ratchet.exe`, so it shows in Explorer and the taskbar.

**Running outside the MSYS2 shell.** `target/release/ratchet.exe` needs the
GTK DLLs. To launch it from Explorer or PowerShell, add
`C:\msys64\ucrt64\bin` to your `PATH`:

1. Open *Settings → System → About → Advanced system settings → Environment
   Variables*.
2. Under *User variables*, edit `Path` and add `C:\msys64\ucrt64\bin`.
3. Sign out and back in, or open a new terminal.

If the build fails with `pkg-config` or "library not found" errors, check
that you are in the **UCRT64** shell (the prompt shows `UCRT64`).

</details>

<details>
<summary><b>🐛 Debugging and troubleshooting</b></summary>

<br>

Run with `RATCHET_DEBUG=1` to enable debug tools (a pen input overlay and an
event log):

```sh
RATCHET_DEBUG=1 cargo run
```

On Windows (MSYS2 shell), use the same command. In PowerShell:
`$env:RATCHET_DEBUG=1; cargo run`.

On Windows, Ratchet draws on the GPU when the machine has one graphics
adapter and in software when it has more (two GPUs, or a virtual one such as
Parsec's), where the GPU path flickers in fullscreen. `RATCHET_GPU=1` or
`RATCHET_GPU=0` forces either; `RATCHET_UI_SCALE` overrides the UI scale.

Pen pressure on Windows goes through GTK's Windows Ink / WinTab support,
which works differently from Linux. Please
[open an issue](https://github.com/R3ktline/Ratchet/issues) with how
pressure, side buttons and the eraser end behave on your device.

</details>

## 🤝 Development

```sh
cargo test --release
cargo clippy
```

Release builds (Linux AppImage, Windows installer) are made by
`.github/workflows/release.yml` from the scripts in `packaging/` when a
`v*` tag is pushed. Bug reports and pull requests are welcome.

## 📄 License

Ratchet is licensed under
[Creative Commons Attribution-NonCommercial-ShareAlike 4.0](LICENSE)
(CC BY-NC-SA 4.0). In short:

- ✅ You may use, share and modify it for **non-commercial** purposes.
- ✅ You must give credit and link back to this project.
- ✅ If you share a modified version, it must use the **same license**.
- ❌ Commercial use is not permitted.

See [LICENSE](LICENSE) for the full terms.
