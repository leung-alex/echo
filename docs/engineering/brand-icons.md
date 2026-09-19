# Echo brand icons

The 2026-09-10 Taskbar devpack supplies the original-based full-color PNG/ICO
artwork. Preserve its dark blue tile and transparent outer corners. The SVG is
an editable flat reconstruction; it is not the raster artwork's original source.

## Consumers

| Entry | Source |
| --- | --- |
| Main EXE, portable and installed Echo.exe | `apps/desktop/icons/icon.ico`, resource group 1 |
| Tray | Main EXE group 1 via `LoadIconMetric(LIM_SMALL)` |
| Native AppWindow, including settings/About and Quick Insert | `echo-taskbar-256.png`, compiled by Slint |
| About content | `echo-256.png` at 48 logical pixels, preserving aspect ratio |
| NSIS installer and uninstaller | `APP_ICON`, the absolute path to the same main ICO |
| Installed-apps listing | Quoted installed `Echo.exe` path, icon index 0 |
| Start menu and optional desktop shortcut | Installed `Echo.exe`, icon index 0, installation working directory |
| User-pinned taskbar entry | Installed EXE/shortcut; pinning remains user-controlled |

The taskbar ICO alias has identical bytes to the main ICO. It is not embedded
as a second resource group. Alternative monochrome tray assets are retained as
source assets only. Functional glyphs and Saved Item icon choices are unchanged.
The retired offscreen `CardSnapshot` component has been removed from production
UI compilation. Existing window visibility, focus and application identity
policies are unchanged.

The main ICO SHA-256 is
`d471ffee2a44a7731dd8c7d65c914545e4eff2de0bbd94b4c051a00a270ead68`.
The runtime 256px PNG SHA-256 is
`a999db15ea6e5b61b49d63d3089ed8359e4eea885e26fa0583d9e12ff388f01d`.

## Installer behavior

Interactive installation offers **Create desktop shortcut**, checked by default
on first install. Upgrades load the previous choice from `install-mode.ini`.
`/DESKTOPSHORTCUT=0|1` overrides that initial choice and also works silently;
an interactive selection can subsequently change it. Other values are rejected.

The `[shortcuts]` section records `desktop_choice`, `desktop_owned` and
`start_menu_owned` separately. Shortcut target checks use IShellLinkW/IPersistFile
without invoking Resolve. Unknown targets, unreadable links and reparse points
are preserved. A legacy link targeting this exact installed EXE can be adopted.
Uninstall requires both ownership and a matching current target; another
installation's application registration is preserved.

`/NOINTEGRATION=1` bypasses the option page and all shortcut/registration changes.
The corresponding uninstaller also skips them. Uninstall still uses the existing
file inventory and non-recursive directory removal, preserving unrelated files
and clipboard data.

## Validation boundary and remaining acceptance

Implementation baseline: `e9ad71d732cb19cb4db8ec19a129863b24ccb70b`, plus the
uncommitted brand changes. UI preview uses an isolated, capture-disabled
synthetic fixture. No source changes were committed or pushed by this task.

- PASS: overlay source hashes were checked during the targeted asset import.
- PASS: `cargo build -p echo-desktop --locked` for the native debug UI preview.
- Observed: About Echo is reachable from Storage & diagnostics; the native
  preview shows the new brand beside Echo and retains AboutSlint/notices.
- PASS: user accepted v7 native UI on 2026-09-10, including image-row switch stability. Temporary geometry tracing was removed after acceptance.
- PASS: devpack asset/frame validator (2,273 checks) and hashes of all 15 imported overlay files; self-check and format check.
- PASS: full `echo.cmd verify`, including new Favorites clear tests. The opt-in 1k/10k fuzzy-search scale test remained ignored by the canonical gate.
- NOT_RUN: release/software-only builds, portable and NSIS packaging.
- NOT_RUN: installed EXE/uninstaller resource extraction, install/upgrade/
  uninstall scenarios, tray recovery and taskbar/DPI acceptance.

Local preview evidence: `.local/ui-review/brand-icons-20260910/`, including
`about-native.png`, `about-uia.txt`, `process.json`, and process logs. The preview
executable is a debug build, not a distributable release. It can be closed with
Quit Echo using its isolated `ECHO_DATA_DIR`.

After UI confirmation, run the canonical gates and the devpack's asset validator.
Exercise normal and silent installs, both desktop choices, upgrade preference
retention, foreign/unreadable/replaced shortcuts, legacy links, paths containing
spaces/Chinese, `/NOINTEGRATION=1`, and uninstall ownership in an isolated account
or VM. Separately record TB01-TB09 from the devpack: running and pinned states,
Quit/relaunch, grouping, Quick Insert, available DPI/themes, upgrades and portable
relocation. Never infer these results from the UI preview or successful builds.

API references: [LoadIconMetric](https://learn.microsoft.com/en-us/windows/win32/api/commctrl/nf-commctrl-loadiconmetric)
and [NSIS CreateShortCut](https://nsis.sourceforge.io/Reference/CreateShortCut).

## Card and Favorites refinements

History/Favorites headers use their system icons; saved rows display only configured icons. Red trash actions clear unpinned History or Favorites (shared saved items remain in other spaces). Cards retain their resident page and scroll for motion snapshots. Actual-height resident-page layout avoids virtual average-height scroll corrections after handoff. The resident page remains bounded to 500 rows; large-page performance is not yet accepted.
