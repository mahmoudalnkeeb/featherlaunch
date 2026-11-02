# Changelog

The notes for each version are published with its release, so write them for people using the
launcher. Add a section here before tagging; the release job stops without one.

## 1.4.2 - 2026-11-02

### Fixed

- Starting the game from a desktop shortcut no longer signs in twice, which could end in a
  "429 Too Many Requests" error.
- If Minecraft limits your sign-ins, the launcher waits and tries once more, then tells you what to
  do instead of showing the raw error.

### Changed

- Desktop shortcuts now use the instance's icon. Make the shortcut again to update an old one.

### Known issues

- If you close the launcher while a game is running, that session's play time isn't saved. Its log
  is still under Saved logs.
- CurseForge packs can't be imported yet.
- Turning off a data pack that is a folder, not a zip, has no effect in game.
- A desktop shortcut opens a new copy of the launcher, even if one is already running.
- On a brand new instance, joining a server waits until you dismiss Minecraft's first-run screen.

## 1.4.1 - 2026-11-02

### Changed

- The game now starts with `javaw.exe` when your Java has it. Wrapper commands and a Java you picked
  yourself work as before.
- Desktop shortcuts are now made by Windows directly, without PowerShell.
- The Windows program now shows its name, publisher and description in its file details, and states
  that it doesn't need admin rights.

### Known issues

- If you close the launcher while a game is running, that session's play time isn't saved. Its log
  is still under Saved logs.
- CurseForge packs can't be imported yet.
- Turning off a data pack that is a folder, not a zip, has no effect in game.
- A desktop shortcut opens a new copy of the launcher, even if one is already running.
- On a brand new instance, joining a server waits until you dismiss Minecraft's first-run screen.

## 1.4.0 - 2026-11-02

### Added

- Tick the box beside a mod to add it to your list, without opening it first. It picks the newest
  stable version. Open the mod to choose a different one.
- Mods you already have now show a check and an Installed badge while you browse.

### Changed

- Search results are now a grid, so more fit on screen, and details open in a popup.
- The skin preview now shows the outer layer standing out from the body, as it does in game.

### Fixed

- Semi-transparent parts of a skin's outer layer no longer hide what's under them in the preview.

### Known issues

- If you close the launcher while a game is running, that session's play time isn't saved. Its log
  is still under Saved logs.
- CurseForge packs can't be imported yet.
- Turning off a data pack that is a folder, not a zip, has no effect in game.
- A desktop shortcut opens a new copy of the launcher, even if one is already running.
- On a brand new instance, joining a server waits until you dismiss Minecraft's first-run screen.

## 1.3.0 - 2026-11-02

### Changed

- Pick everything you want first, then install it all at once. Add mods, resource packs, shaders and
  data packs to a list as you browse, check it with Review, and press Install all. Required
  dependencies come along, and one failed download doesn't stop the rest.
- Leaving an instance with items on your list asks first.

### Known issues

- If you close the launcher while a game is running, that session's play time isn't saved. Its log
  is still under Saved logs.
- CurseForge packs can't be imported yet.
- Turning off a data pack that is a folder, not a zip, has no effect in game.
- A desktop shortcut opens a new copy of the launcher, even if one is already running.
- On a brand new instance, joining a server waits until you dismiss Minecraft's first-run screen.

## 1.2.0 - 2026-11-02

### Added

- Join a server when the game starts, from an instance's Settings or the new Servers list.
- Servers and Screenshots are in the Worlds tab. The server list shows who's online, and you can
  add, remove and join servers from it. A server that relies on an SRV record shows as offline, but
  you can still join it.
- Notes on each instance, in its Settings.
- Modpacks with optional files now ask which ones you want.
- Move your instances to another folder or drive from Settings.
- Create a desktop shortcut that starts an instance, from its More menu.
- Start the launcher from the command line with `--launch`, `--server`, `--world`, `--profile`,
  `--offline` and `--dir`.
- Pick which account an instance plays with.
- Run a wrapper command, commands before launching and after exiting, and set environment
  variables, for all instances or just one.
- On Linux: GameMode, MangoHud, the discrete GPU, and the system's GLFW and OpenAL.
- Sort Modrinth search and filter it by category.
- Choose whether mod updates can be betas and alphas.
- A proxy setting for the launcher's own downloads.
- Pick a cape, and keep a list of saved skins, on the Accounts page.
- Show what you're playing on your Discord profile. This is off until you turn it on in Settings.

### Changed

- Instance zips never bring launch commands, a join server or a chosen account with them, because
  those can run commands or belong to the PC they came from.

### Known issues

- If you close the launcher while a game is running, that session's play time isn't saved. Its log
  is still under Saved logs.
- CurseForge packs can't be imported yet.
- Turning off a data pack that is a folder, not a zip, has no effect in game.
- A desktop shortcut opens a new copy of the launcher, even if one is already running.
- On a brand new instance, joining a server waits until you dismiss Minecraft's first-run screen.

## 1.1.0 - 2026-11-01

### Added

- Modpacks can update in place. Instance Settings shows the installed version, a list of versions
  with their changelogs, and Update from file. Your worlds, `options.txt` and `servers.dat` stay as
  they are. Instances made before this version can't update this way.
- Import now takes instance zips from Prism Launcher, MultiMC and FeatherLaunch, and can copy every
  instance from a Prism Launcher or MultiMC folder.
- Export an instance from its new Export tab, as a `.zip` for backups or a `.mrpack` for sharing.
- The Worlds tab can import a world from a zip, and copy, rename or delete worlds. Each world has
  data packs you can add, turn off and browse on Modrinth.
- Saved logs: the Log tab now lists an instance's past logs and crash reports.
- Upload a log to mclo.gs and get a link to share. Access tokens and your home folder are hidden
  first.
- Add mods, resource packs and shaders with the Add files button or by dropping them on the window.
- Mods that aren't on Modrinth now show the name and version from inside their file.
- Set the game window size, fullscreen and minimum memory, for all instances or just one.
- Alpha and beta versions of Minecraft can now be installed.

### Changed

- New installs pick a memory setting that suits your PC, between 1 and 3 GB. Your saved setting
  doesn't change.
- Launching asks first when your PC is low on free memory. You can turn this off in Settings.
- If the Java you chose can't run a Minecraft version, that launch uses automatic Java and tells
  you. You can turn this off in Settings.
- Importing a local modpack or instance zip now lists any mods Modrinth hasn't reviewed and asks
  before installing them.
- Worlds show their in-game name instead of their folder name.
- The Crash reports button on the Log tab is gone, because crash reports are now under Saved logs.

### Known issues

- If you close the launcher while a game is running, that session's play time isn't saved. Its log
  is still under Saved logs.
- CurseForge packs can't be imported yet.
- Turning off a data pack that is a folder, not a zip, has no effect in game.

## 1.0.0 - 2026-10-30

### Changed

- FeatherLaunch is now open source under the GNU General Public License, version 3.
- There are no changes to how the launcher works in this version.

## 0.3.0 - 2026-10-30

### Changed

- A cleaner layout throughout: less space goes on headers, more on your instances and mods, and
  every page now fits at the smallest window size.
- The sidebar has four rows. Custom versions, Modrinth modpacks and file imports are together under
  New instance, and the folder shortcuts moved to Settings.
- The selected instance takes less room in the Library, and its actions are Play, Manage and a More
  menu. Group and loader filters can now be combined.
- On an instance's page, Mods comes first and Settings last.
- New instance starts with the latest release already picked.
- Fewer icons: most buttons and menus are now plain text.

### Added

- Settings show a save bar only when something has changed, and leaving an instance with unsaved
  changes asks first.
- Modpack results now show their details beside the list, and you can page through all of them.

### Fixed

- If something can't be loaded, such as the version list when you're offline, the page says so and
  offers Try again instead of spinning forever.
- Deleting a mod and removing an account now ask first.
- Text is no longer stretched in mod descriptions and in Activity errors.
- The task button no longer overlaps your account name at the smallest window size.

### Known issues

- If you close the launcher while a game is running, that session's play time isn't saved and its
  console output is lost.

## 0.2.2 - 2026-10-30

### Fixed

- Closing the launcher while a game is running no longer freezes it. The launcher now closes right
  away and the game keeps running.
- Windows: dragging the window to the top of the screen maximizes it again, and dragging it to a
  side or corner snaps it to half or a quarter of the screen.
- Windows: a maximized window can be dragged back down by its title bar.

### Known issues

- If you close the launcher while a game is running, that session's play time isn't saved and its
  console output is lost.
