# Changelog

The notes for each version are published with its release, so write them for people using the
launcher. Add a section here before tagging; the release job stops without one.

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
