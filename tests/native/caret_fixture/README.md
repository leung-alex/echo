# Owned TSF caret fixture

`CaretTsfFixture.cs` is an isolated WPF editor with synthetic text only. It
owns its window and a local named pipe whose name is published in
`fixture.ready.json`. The runner sends only the package command vocabulary and
uses `fixture.oracle.json` as the independent layout oracle. It never targets a
user window or reads a user document.

The WPF text store is allowed to create and manage its own TSF context. This is
fixture setup; the injected Echo observer still uses the target thread's
existing manager and never activates TSF.
