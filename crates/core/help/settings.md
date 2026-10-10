# Settings

Open **Settings** with ⌘, (Ctrl+, on Linux and Windows), or from the app menu on macOS (File menu elsewhere). Changes apply as soon as you make them, and each setting has a **Reset** button that puts it back to its default.

There are two kinds of settings:

- **App preferences** are yours, on this machine: they apply to every workbook you open. They live in a text file, `settings.toml`, in your config folder (`~/Library/Application Support/wbs` on macOS, `~/.config/wbs` on Linux). The settings window shows the exact path. The file is only written when you change something.
- **Workbook settings** are saved in the workbook file, so they travel with it. Changing one is an unsaved change to the workbook, like editing a cell.

You can edit `settings.toml` by hand. A value the app can't use is never rewritten or dropped: the settings window shows the setting, says what's wrong with the stored value, and the default is used until you fix it or press **Reset**. Settings this version doesn't know about (from a newer version, say) are kept as they are, in the file and in workbooks. Reset removes the setting from the file, so it follows the default.

## Every setting

{{settings}}
