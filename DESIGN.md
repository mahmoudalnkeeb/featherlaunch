# FeatherLaunch design system

FeatherLaunch should feel like a solid classic desktop app: quiet, dark and focused on your
games. Prism was the starting point for features, not for looks. The look is a near-black base,
outlined surfaces with small corners, rectangular controls, an icon rail and a single cool
accent. Every screen and popup uses the same few shapes, so nothing looks like it came from a
different app.

Everything here has to run on an Intel HD 4600 at near-0% idle CPU, so every rule is also a
performance rule.

## Toolkit

FeatherLaunch stays on **egui** (eframe, `glow` backend). Options considered:

| Toolkit | Rendering | Idle RAM (typical) | Fit for this project |
|---|---|---|---|
| **egui** (current) | OpenGL 2/3 via glow, immediate mode | ~45 MB | Lightest option that runs on HD 4000/4600. Fully custom painting, custom title bar via viewport commands. |
| Slint | Software, FemtoVG (OpenGL) or Skia | ~30–60 MB | Good runner-up: declarative `.slint` files, easy styling. Costs a full UI rewrite and its royalty-free license needs attribution. |
| iced | wgpu (DX12/Vulkan), tiny-skia fallback | ~80–120 MB | Clean Elm architecture, but wgpu is heavy for old iGPUs. |
| Freya | Skia | ~80 MB+ | Nice visuals; Skia adds ~15 MB to the binary. |
| GPUI (Zed) | DirectX 11 / Metal | ~100 MB+ | Beautiful, but young API and tied to Zed's needs. |
| Tauri / Dioxus desktop | WebView2 | 150–300 MB | Web styling is easy, but it breaks the low-end goal. |

egui's default look is the problem, not egui itself. The fix is to own the look: our own tokens,
painted widgets, vector icons and window chrome, built on egui's painter.

## Principles

1. **Content first.** Instances are the content. Chrome (title bar, sidebar) is thin and dark, and
   surfaces stay neutral. Color comes from instance icons and the art on cards, not from panels.
2. **One accent.** Cyan marks the one primary action on screen (Play, Install, Create) and the
   current selection. Nothing else is cyan: no captions, labels or empty-state glyphs.
3. **Surfaces with outlines.** Depth comes from three surface steps plus a 1 px `BORDER` around
   cards, dialogs, inputs, secondary buttons and chips. No shadows.
4. **Rectangles everywhere.** No pills. Buttons, chips, inputs and the search field are
   rectangles with `R_MD`; cards, dialogs and the hero use `R_LG`. Tabs are underlined.
5. **Calm when idle.** No animation loops and no polling. The UI redraws only on input,
   background events, or a one-off timer (toasts).
6. **Words before icons.** Buttons and menus are text. Icons are for navigation, window controls,
   Play and Stop, and row actions that repeat down a list. An icon never repeats the text beside it.
7. **Fits at every size.** Every page works from the 760×480 minimum to a maximized window, with
   nothing clipped and the main action on screen. Check both sizes after changing a layout.

## Color tokens

All colors live in `src/ui/theme.rs`. Never write a raw `Color32` in a view.

| Token | Hex | Use |
|---|---|---|
| `BG` | `#0E0F11` | Window base, title bar, rail |
| `SURFACE` | `#16171A` | Cards, dialogs, hero, panels |
| `RAISED` | `#1E2024` | Inputs, secondary buttons, chips |
| `HOVER` | `#26282D` | Hover state for anything on `SURFACE` or `RAISED` |
| `BORDER` | `#26282D` | 1 px separators and container outlines (cards, panels, dialogs, window) |
| `TEXT` | `#ECEDEF` | Titles and body |
| `TEXT_MUTED` | `#9095A0` | Secondary lines, labels, hints, placeholders, section captions |
| `TEXT_FAINT` | `#5E626B` | Outlines of controls (inputs, secondary buttons, chips, toggles), disabled text, decorative glyphs. Never readable text: it is under 4.5:1. |
| `ACCENT` | `#74DDE8` | Primary buttons, selection, focus, progress |
| `ACCENT_HOVER` | `#9AE8F0` | Primary button hover |
| `ON_ACCENT` | `#0B1214` | Text and icons on `ACCENT` |
| `OK` | `#5DD39E` | Running dot, updates available |
| `WARN` | `#F2C46D` | Warnings |
| `ERR` | `#F07178` | Errors, destructive actions |
| `CLOSE` | `#E5484D` | Window close button hover |

Loader tints (placeholder icons, card art): Vanilla `#5FA35A`, Fabric `#C2A57A`,
Quilt `#9A6BD0`, Forge `#5B7CB8`, NeoForge `#E08A45`.

## Typography

- Family: **Segoe UI** (regular) and **Segoe UI Semibold**, loaded from `C:\Windows\Fonts` at
  startup. Nothing is embedded, so the exe does not grow. egui's built-in fonts stay as the
  fallback (and for symbols).
- Scale (px), as functions in `theme.rs`: `display()` 22 semibold (page titles), `title()` 16
  semibold, `body()` 14, `body_strong()` 14 semibold (row titles, primary buttons), `label()` 13
  (secondary lines), `caption()` 11 semibold uppercase with letter spacing (section labels).
  Mono 12.5 is the console. Views use these names, never a size of their own.
- Text color carries hierarchy: `TEXT` for primary, `TEXT_MUTED` for everything secondary,
  including hints. Don't use bold for emphasis in body text.

## Spacing and shape

- 4 px grid. Steps in `theme.rs`: `S1` 4, `S2` 8, `S3` 12, `S4` 16, `S6` 24.
- Page padding: 24 at the sides and bottom, 16 at the top (`widgets::page_frame`). Gap between
  cards 12. Between list rows 4. Inside cards 16.
- Forms stop at 640 px (`FORM_WIDTH`), so fields and their save bar stay together when maximized.
- Radii: `R_SM 2` (badges, menu rows, toggles), `R_MD 3` (buttons, inputs, chips, list rows,
  icons), `R_LG 4` (cards, dialogs, panels, hero). Progress bars are square.
- Control heights: `H_MD` 32 (default), `H_LG` 36 (Play and the buttons beside it), `H_SM` 28
  (compact, in lists).

## Layout

```
┌──────────────────────────────────────────────────────────────┐
│ ◆ FeatherLaunch     [task progress]       (head) Name ▾  — ▢ ✕ │  title bar 40
├──────────┬───────────────────────────────────────────────────┤
│ Library  │ Library                                 [ search ]│
│ New inst │ (All 7) (Modded 4) (Survival 1)      [All loaders]│
│          │ ┌───────────────────────────────────────────────┐ │
│          │ │ icon · name · details   [▶ Play] [Manage]  ⋯ │ │
│          │ └───────────────────────────────────────────────┘ │
│          │ ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐                  │
│ Accounts │ │card │ │card │ │card │ │card │   instance grid  │
│ Settings │ └─────┘ └─────┘ └─────┘ └─────┘                  │
│ Collapse │                                                  │
└──────────┴───────────────────────────────────────────────────┘
 sidebar 180
```

- **Title bar (40 px):** the app mark and name on the left, the account chip and window controls
  on the right, and a 1 px `BORDER` line under all of it. The rest is a drag area, with the
  activity button in the middle, moved over or narrowed to stay clear of the chip.
- **Activity (`src/ui/activity.rs`):** while anything runs, the title bar button shows a spinner,
  the first task's title and percent, "+N more", and the overall progress as a bar on its bottom
  edge; afterwards it shows "All tasks done" or "N tasks failed". Clicking it opens a dropdown
  with one card per running task (title, current step, progress bar, files done / total, bytes,
  speed, elapsed and estimated time left, files downloading now) and a Recent list of the last 8
  finished tasks, failed ones with their error. Byte counts don't wake the UI per chunk; a
  250 ms redraw timer runs only while a task is running.
- **Cancel:** installs, updates, modpack imports and launches have a Cancel button on their card
  (`App::cancellable_task`). Cancelling drops the task at its next await, so these tasks clean up
  through drop guards (partial `.part` files, a half-made modpack instance, a launch's reserved
  slot), and an old mod version is removed only after its replacement is on disk. Steps that a
  dropped future can't stop (extracting Java, running the Forge/NeoForge installer, applying
  modpack overrides) hold a `Progress::critical()` guard: the button is hidden and the card says
  the step will finish first.
- **Sidebar (180 px):** one labelled row per page (glyph + text, 36 px high), on `BG` with a 1 px
  `BORDER` right edge. Library and New instance are at the top, Accounts and Settings at the
  bottom. The active row is `RAISED`, semibold, with an accent bar on the sidebar's left edge.
  Labels, not tooltips, name each page, as the Modrinth App and Heroic Games Launcher sidebars do.
  A "Collapse sidebar" row at the very bottom shrinks it to a 56 px icon rail (labels become
  tooltips); the choice is saved in `config.json`.
- **Launcher update:** when a newer version is out, an "Update to X.Y.Z" row with an `OK` dot sits
  above Accounts. Clicking it runs a normal Activity task, and the row then reads "Restart to
  update". Restarting waits for running games and tasks; closing the launcher also finishes the
  update. There is no polling: it checks once on start, or from Settings.
- **Pages, not popups.** Everything the user works in is a page in the main area: Library, New
  instance, Accounts and Settings. Each page opens with a 22 px title and a muted line, uses the
  same margins and fills the width (`widgets::page`). Pages keep their state when you switch away
  (a half-filled form, a settings draft) until saved or used. Pages with fixed columns, such as New
  instance, don't scroll as a whole: their lists do, so the main action stays on screen.
- **Library page:** header, group chips with a Loader dropdown on the right (they combine), then
  a bar for the selected instance, then the grid. Columns share the full width, so the grid lines
  up with the bar. An instance hidden by the search or filters leaves the bar too.
- **Instance page:** opens in place of the library, with a back button (or Esc), a one-row header
  with icon and name, underlined tabs (Mods, Resource packs, Shaders, Worlds, Log, Settings) and
  the Play button. "Manage" opens Mods, or Worlds for an instance with no mod loader. The editor
  and console are pages, not floating windows.
- **New instance page:** a segmented control picks Custom version, Modrinth modpacks or Import
  file. Custom has the version list on the left, filling the height, and the form on the right,
  with the latest release already picked.
- **Modrinth search (`src/ui/pages/project.rs`):** Browse and Modpacks share a results list and a detail
  pane with the install controls in a footer that stays on screen. Under 640 px the detail replaces
  the list, with a "Back to results" button.
- **Forms:** a 160 px label column and a control (`widgets::form_row`), with sections under a
  caption. A save bar is fixed to the bottom only while the form differs from what is saved.
  Leaving an instance page with unsaved edits asks first.
- **Failed loads:** anything loaded for a page (version lists, search, project versions) reports
  failure to the page, which shows a notice with "Try again" instead of a spinner.
- **Dialogs (modals):** only for short blocking steps: the Microsoft sign-in code, errors, and
  confirmations before something can't be undone (`src/ui/dialogs.rs`): deleting an instance or a
  file, removing an account, discarding edits. They are centered `SURFACE` boxes with `R_LG`, a 1 px `BORDER` outline and a black 55%
  backdrop. Each has a title bar strip on `BG` (title left, ✕ right) separated from the body by
  a `BORDER` line; the body has 20 px padding. Esc or the ✕ closes them; a backdrop click doesn't, so drafts are never lost.
- **Toasts:** short status messages appear bottom-right for 4 seconds, then disappear. Failures
  have an `ERR` dot and stay for 8 seconds.

## Components

| Component | Spec |
|---|---|
| Primary button | `R_MD`, `ACCENT` fill, `ON_ACCENT` semibold text, hover `ACCENT_HOVER`. One per view. Danger replaces it where the action is destructive. |
| Secondary button | `R_MD`, `RAISED` fill, 1 px `BORDER`, `TEXT`; hover `HOVER` fill and `TEXT_FAINT` border. |
| Ghost button | No fill, `TEXT_MUTED`, hover `HOVER` fill and `TEXT`. |
| Danger button | `R_MD`, `ERR` at 15% fill, `ERR` text. |
| Icon button | 32×32, `R_MD`, glyph in `TEXT_MUTED`, hover `HOVER` + `TEXT`. Always has a tooltip. |
| Chip | `R_MD`, 28 high, `RAISED` with a `TEXT_FAINT` outline; selected is an `ACCENT` tint and outline. An optional count sits after the label in a darker `R_SM` box. |
| Tabs | For going to a view. Text tabs on a 1 px `BORDER` baseline as wide as the page; the selected tab has a 2 px `ACCENT` underline and `TEXT`, others `TEXT_MUTED`, hover `HOVER`. |
| Segmented control | For picking a value or a mode. A joined `RAISED` row with a `TEXT_FAINT` outline; the selected segment is `HOVER` with `TEXT`. |
| Dropdown | egui's ComboBox with the chevron glyph (`widgets::combo`). Where the selection can be long, such as a version name or a path, use `widgets::combo_cut`, which holds its width. |
| Slider | A 200 px rail and a small handle, with the value in a box beside it (`widgets::slider`). |
| Search field | `R_MD`, `RAISED`, `TEXT_FAINT` outline (accent on focus), magnifier glyph, hint in `TEXT_MUTED`. |
| Text input | `RAISED`, `R_MD`, `BORDER` outline, `TEXT_FAINT` outline on hover, accent on focus. |
| Toggle | 34×20 `R_SM` track (`RAISED` + `TEXT_FAINT` outline off, `ACCENT` tint on) with a square knob, `ACCENT` when on. It is quieter than a primary button because lists show a column of them. |
| Card (instance) | At least 156 wide, 196 high, `SURFACE`, `R_LG`, 1 px `BORDER`. Top art area is a loader tint with the instance's 72 px icon, or its first letter directly on the tint. Name in `body_strong`, version and loader in `label`. Hover goes to `RAISED` and shows a square Play button on the art. Selected has a 1.5 px `ACCENT` outline. A running instance shows a Running badge. |
| Hero | 96 high, `SURFACE` with a 1 px `BORDER`. 64 px icon, name in `display`, details in `label`, then Play, Manage and a More menu on the right. A running instance shows an `OK` dot and "Running" in the details. |
| List row | `widgets::list_row`. 52–68 high, `SURFACE`, `R_MD`, hover `RAISED`, selected `ACCENT` at 12% with a 1 px accent outline. |
| Badge | `R_SM`, 18 high, Caption text, tinted fill (color at 18%) with colored text. Data such as a version number keeps its case (`badge_exact`). |
| Notice | A line tinted with `WARN` or `ERR`, with an optional button that fixes the problem. |
| Empty state | Centered title and a muted line, with at most one button (`widgets::empty_state`). |
| Progress | 4 px square track in `RAISED`, `ACCENT` fill. |

## Accessibility

- Readable text is at least 4.5:1 on its surface: `TEXT` and `TEXT_MUTED` pass on every surface,
  `TEXT_FAINT` doesn't and is kept for disabled text and decoration.
- Control outlines are `TEXT_FAINT` (3:1 on `BG`), so inputs and buttons stay findable.
  `BORDER` is only for containers.
- Every painted widget draws a 1.5 px `ACCENT` focus ring when it has keyboard focus. Tab moves
  focus; Enter or Space activates.
- Icon-only buttons always have a tooltip, which is also their accessible name.
- Text cut with "…" shows the full text as a tooltip.
- Screen readers: the accessible names are set, but eframe's AccessKit bridge is not compiled
  in (it adds size and RAM). Turn on eframe's `accesskit` feature to expose them.

## Icons

Icons are drawn as vector shapes by `src/ui/glyph.rs` (strokes, circles, polygons), not
emoji. That keeps them crisp at any DPI, colored by the theme, and free in binary size. Stroke
width is 1.5 px on a 16 px box, scaled with the box. Set: library, plus, folder, gear, search,
play, stop, trash, more, back, forward, close, minimize, maximize, restore, chevron-down, check,
user, refresh, download.

## App icon

A single feather in `ON_ACCENT` ink on an `ACCENT` tile: the name, and the promise that the
launcher weighs almost nothing. It is drawn on a 24-unit grid: tile radius 5.5, the vane as two
cubic curves from (7, 17) to (19.5, 4.5), a shaft and two barb splits cut out in the tile color,
and a short stem. No gradients or detail, so it stays readable at 16 px.

- `assets/make-icon.ps1` renders `assets/icon.ico` (16–256 px) and the PNGs from that geometry.
- `build.rs` embeds the ICO in the exe; `main.rs` sets the window and taskbar icon from
  `icon-256.png`; `glyph::app_mark` draws the same shape in the title bar.

## Window chrome

- The native title bar is turned off (`with_decorations(false)`), and the app draws its own.
- Drag: pressing on an empty part of the title bar sends `ViewportCommand::StartDrag`. Windows
  then handles moving and Aero Snap. Double-clicking toggles maximize.
- Resize: a 6 px band around the window edges sets the resize cursor and sends
  `ViewportCommand::BeginResize(direction)` on press. It is off while maximized.
- Controls: minimize, maximize/restore and close. 46×40 hit areas like native Windows. Hover
  is `HOVER`; close hover is `CLOSE` with white glyph.
- A 1 px `BORDER` outline frames the window when it is not maximized, so it stays visible on
  dark desktops without a native shadow.
- Intel HD 4000/4600 OpenGL drivers draw a borderless window off by the size of any native frame
  style it still has: about 30 px for `WS_CAPTION`, 8 px for `WS_THICKFRAME`. The app removes
  both every frame and keeps a maximized window inside the monitor's work area
  (`chrome::fix_window_style`). Windows needs `WS_THICKFRAME` to start a resize or a drag that can
  snap, so `chrome::drag_wndproc` sets it around the start of each one, outside any frame: a frame
  presented with it lands off too. Check the window edges on Intel graphics after touching window
  setup.

## Performance rules

- No `request_repaint()` in view code except one-off timers (`request_repaint_after`).
- No shadows, and no gradient meshes.
- Icons are 64×64 textures, loaded lazily and only while visible.
- Fonts come from the system. Never embed font files.
- Dialog backdrops are a single rectangle.
