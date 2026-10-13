# Ratchet — working rules

Pen-first notes app, Rust + GTK 4 + libadwaita. Main dev target right now is
**Windows at 150% display scaling**; it must look as clean there as on Linux.

## How to work

- Edit files only with the Edit / Write tools — never Python, sed or other
  scripts.
- Build and run on Windows with the MSYS2 UCRT64 toolchain:
  `export PATH=/c/msys64/ucrt64/bin:$PATH && cargo build --release`, then
  `./target/release/ratchet.exe`. Close a running Ratchet first (it holds
  the exe, and writes settings on exit).
- Run `cargo test --release` after changes to settings or models.
- On Windows, GTK only renders on the GPU with Direct Composition, which
  `platform::early_setup()` turns on (`GDK_DEBUG=dcomp`). Without it GTK
  falls back to the software Cairo renderer and animations stutter. Keep it
  on; `GDK_DEBUG=` (empty) turns it off for testing.

## UI/UX rules (follow for every UI change)

These come from the owner's feedback. A change that breaks one is not done.

### 1. Nothing is ever cramped
- Nothing touches the edge of its container. Bars, header bars, popovers,
  dialogs and clusters all have inner padding (bars: 8 px top/bottom,
  12 px left/right; popovers: ~12 px).
- Neighboring controls are at least 8 px apart; separate clusters 16 px.
  Buttons inside a cluster 4 px apart.
- Buttons in a header bar are padded like the rest. A text button (Cancel,
  Export…) gets ~16 px horizontal padding and is never flush with the
  window edge.
- Menu rows are at least 34 px tall, with space between the radio/check,
  the label and the shortcut.
- If something looks squeezed, increase spacing before shrinking anything.
- These sizes live in `load_sizes()` in `src/main.rs` and apply to **every**
  header bar, dialog and menu. Never style only the main window and forget
  dialogs.

### 2. Scale everything
- GTK on Windows renders at 1x, and `platform::ui_scale()` enlarges fonts
  and icons. Every fixed size set in code goes through `platform::px(..)`,
  and every size in CSS goes through `load_sizes()`. No raw pixel numbers
  for widths, heights, spacing or margins.
- Never use `follows_content_size(true)` on dialogs; give them a
  `content_width(px(..))` wide enough for their content.

### 3. Nothing gets cut off
- Text that ellipsizes ("Export “…", "Split in…") means the layout is
  wrong. Fix the width or the copy, don't accept the "…".
- The window buttons (minimize/maximize/close) must never be pushed out of
  the window. Bars must not ask for more width than the window has: the
  window's minimum width comes from measuring the bar.

### 4. Short copy, no walls of text
- Titles are 1–2 words. Don't put names in dialog titles; show them in
  the content.
- Row subtitles only when under ~30 characters. Explanations go in
  `tooltip_text`.
- Dropdown/option labels under ~14 characters.
- No footnotes or long group descriptions.

### 5. Icons and pictures over words
- Rows in dialogs and preferences get a prefix icon. Choices get an icon
  and short label (`adw::ToggleGroup`), or a small picture where that's
  clearer (e.g. page background patterns).
- Only use icons that exist in
  `C:\msys64\ucrt64\share\icons\Adwaita\symbolic`; check before using one.
- Important actions are obvious: labeled button + clear icon (e.g. Export
  uses `send-to-symbolic` + "Export", with an accent tint).

### 6. Wide layouts, not tall ones
- Dialogs and preferences are horizontal: wide windows, content split into
  pages/groups (with icons) rather than one long scroll.

### 7. Group related things
- Related controls sit together in one cluster (`rt-tool-group`): e.g. the
  drawing tools, history (undo/redo), page view (zoom, page setup).
- Header layout: tools on the left, page title in the exact middle
  (`src/title_layout.rs`), history / page view / Export / fullscreen /
  window buttons on the right.

### 8. Minimal by default, extensible
- Start with the least needed: one pen, then eraser, select, pan. More
  pens, highlighters and optional tools (text) are added from the "+"
  menu and tile next to the others.
- Common tools have keyboard shortcuts (e.g. T for text) that never fire
  while typing in a text field.

### 9. Sidebar
- The sidebar is only as wide as its longest entry plus a small margin.
  No large empty space after the text.

### 10. Design language: Notion calm, OneNote ink
- Notion: warm neutral surfaces (colors in `style.css`, light and dark),
  hairline borders instead of heavy boxes, flat buttons that only fill on
  hover, quiet secondary text, soft rounded sidebar rows, page icons.
- OneNote: pens drawn as pens in their ink color, sections marked with a
  colored tab in the notebook's color.
- The accent color is for the few things that should stand out (Export,
  switches), not decoration.
- On Windows nothing may be see-through (it renders black): no shadows,
  no popover arrows, square corners (see `platform.rs`).

### 11. Check it like the owner will
- Look at every screen you touched in a real screenshot. Don't drive the
  owner's mouse/keyboard while they may be using the computer: start the
  app with `RATCHET_SHOW=menu|page|export|prefs` to open that part by
  itself, and capture the screen without sending input.
- Before calling a UI change done, check it at 150% scaling and in a
  narrow window: nothing cramped, nothing cut off, nothing pushed off-screen.
