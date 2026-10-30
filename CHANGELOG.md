# Changelog

The notes for each version are published with its release, so write them for people using the
launcher. Add a section here before tagging; the release job stops without one.

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
