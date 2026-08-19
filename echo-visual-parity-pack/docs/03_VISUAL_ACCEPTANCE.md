# Echo P08 Acceptance

Complete when:
- Quick Insert presentation is React/component-based again,
- original interaction structure is restored,
- original visual language is restored using Echo-local tokens,
- temporary `innerHTML` presentation is removed,
- no Culsans source/UI package dependency exists,
- no unresolved `--echo-*` token remains,
- settings visually match the restored language,
- screenshot evidence covers the required state matrix,
- Echo backend/domain tests remain green.

Run:

```text
.\echo.cmd verify
.\echo.cmd acceptance clipboard
.\echo.cmd acceptance quick-insert
.\echo.cmd package
```

If E07 is incomplete, use equivalent local commands and record them.
