# Independent Windows packaging

`echo.cmd package` builds the native Release executable, a deterministic portable ZIP, hashed payload inventory and dependency notices. If `makensis.exe` is on PATH or installed in standard NSIS paths, it additionally produces a per-user installer. `ECHO_NSIS_EXE` can name an explicitly configured compiler. No Tauri bundler, Node or shared browser runtime is required.

The output path is a new timestamped directory under `target/echo-package/<version>/`; package.json and SHA256SUMS.txt identify artifacts. ZIP timestamps are fixed; identical inputs yield an identical portable archive. Source commit and dirty status are recorded.

The installer defaults to `%LOCALAPPDATA%\Programs\Echo`, owns only its enumerated payload files, and never recursively deletes arbitrary installation content or `%LOCALAPPDATA%\Echo` data. Shared system WebView2 is never uninstalled. For isolated tests, `/S /NOINTEGRATION=1 /D=<fresh absolute test path>` skips shortcut/registry integration.

License text is copied verbatim from resolved packages and deduplicated by SHA256. resolved-licenses.json maps packages to original text. The full lock graph includes build/optional/other-platform dependencies; inventory inclusion does not mean a DLL is shipped. Some published crates include only license metadata; their declared source/license remain in the notices for review. No font files are included.
