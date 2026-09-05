# Matched native performance observations

These Windows PowerShell scripts are the exact semantic UIA/QPC observation algorithm used for the archived A0 baseline. The only collector change is resolving its path relative to this checkout rather than a previous absolute worktree. They require a dedicated authorized Windows test desktop and synthetic fixture directories, never the real Echo database.

`Measure-UiReady.ps1` measures 30 independent process starts and 50 hot handoffs by default, requiring both visible windows, usable search and the expected first content. Includes UIA observation/poll overhead; repeated launches are warm OS-file-cache process starts, NOT boot-cold starts.

`Measure-Resident.ps1` requires D1/D2 fixtures, observes semantic content/visibility, waits 30 seconds, collects 300 seconds by default and checks database signatures. Polling discovery alone does not prove complete process ownership; review metadata, ancestry, executable paths and WebView profile before comparing.

Scripts stop only the isolated product process they start. They do not measure GPU memory or physical input-method compatibility. Use the same script hashes, fixture data, Release mode and machine for both versions. A measurement record is not an automatic gate PASS.
