package main

import (
	"bytes"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func TestEchoFixedWorktreeTable(t *testing.T) {
	want := []fixedWorktree{
		{branch: "codex/foundation", path: `D:\Worktrees\echo\foundation`},
		{branch: "codex/ui", path: `D:\Worktrees\echo\ui`},
	}
	if len(fixedWorktrees) != len(want) {
		t.Fatalf("fixedWorktrees = %#v, want %#v", fixedWorktrees, want)
	}
	for i := range want {
		if fixedWorktrees[i] != want[i] {
			t.Fatalf("fixedWorktrees[%d] = %#v, want %#v", i, fixedWorktrees[i], want[i])
		}
	}
}

func TestSyncSafetyClassifications(t *testing.T) {
	for _, scenario := range []string{"aligned", "behind", "dirty", "ahead", "diverged", "main-changed"} {
		t.Run(scenario, func(t *testing.T) {
			main, target, branch := newSyncRepository(t)
			initial := gitTest(t, main, "rev-parse", "HEAD")
			switch scenario {
			case "behind":
				commitTest(t, main, "main.txt", "main")
			case "dirty":
				mustWrite(t, filepath.Join(target, "dirty.txt"), "dirty")
			case "ahead":
				commitTest(t, target, "branch.txt", "branch")
			case "diverged":
				commitTest(t, target, "branch.txt", "branch")
				commitTest(t, main, "main.txt", "main")
			case "main-changed":
				commitTest(t, main, "main.txt", "main")
			}
			head := gitTest(t, main, "rev-parse", "HEAD")
			if scenario == "main-changed" {
				head = initial
			}
			a := &app{out: io.Discard, errOut: io.Discard}
			_, err := a.syncOne(main, head, fixedWorktree{branch: branch, path: target})
			wantError := scenario == "dirty" || scenario == "ahead" || scenario == "diverged" || scenario == "main-changed"
			if (err != nil) != wantError {
				t.Fatalf("error = %v, wantError = %v", err, wantError)
			}
			if scenario == "behind" && gitTest(t, target, "rev-parse", "HEAD") != gitTest(t, main, "rev-parse", "HEAD") {
				t.Fatal("target did not fast-forward")
			}
		})
	}
}

func TestSyncMenuAndRendering(t *testing.T) {
	t.Run("choices", func(t *testing.T) {
		for _, test := range []struct {
			input string
			want  int
		}{{"1\n", 0}, {"a\n", -2}, {"Q\n", -1}, {"bad\n2\n", 1}} {
			var out bytes.Buffer
			got, err := readSyncChoice(strings.NewReader(test.input), &out, 2)
			if err != nil || got != test.want {
				t.Fatalf("input %q: got %d, %v", test.input, got, err)
			}
			if strings.HasPrefix(test.input, "bad") && !strings.Contains(out.String(), "Invalid choice") {
				t.Fatal("invalid choice was not retried")
			}
		}
		if _, err := readSyncChoice(strings.NewReader(""), io.Discard, 1); err == nil {
			t.Fatal("EOF accepted")
		}
	})

	t.Run("inventory", func(t *testing.T) {
		snapshot := syncSnapshot{
			mainHead:    "1234567890abcdef",
			containsAll: true,
			candidates: []syncCandidate{
				{fixedWorktree: fixedWorktree{branch: "codex/a", path: "a"}, head: "1234567890abcdef", state: syncAligned},
				{fixedWorktree: fixedWorktree{branch: "codex/b", path: "b"}, head: "abcdef0123456789", state: syncSafe},
				{fixedWorktree: fixedWorktree{branch: "codex/c", path: "c"}, head: "fedcba9876543210", state: syncUnsafe, reason: "branch is ahead of main"},
			},
		}
		var out bytes.Buffer
		renderSyncSummary(&out, snapshot, false)
		renderSyncCandidates(&out, snapshot.candidates, false, "Select a branch to fast-forward to main:", true)
		text := out.String()
		for _, want := range []string{"main: 1234567890ab", "[1] codex/a", "[aligned]", "[safe] fast-forward to main", "[unsafe] branch is ahead of main", "[A] All fixed", "[Q] Quit"} {
			if !strings.Contains(text, want) {
				t.Fatalf("missing %q in:\n%s", want, text)
			}
		}
		if strings.Contains(text, "\x1b[") {
			t.Fatal("plain rendering contains ANSI")
		}
		if !strings.Contains(colorize("ok", "32", true), "\x1b[32m") || colorsAllowed([]string{"NO_COLOR="}) || colorsAllowed([]string{"TERM=dumb"}) {
			t.Fatal("color switches are incorrect")
		}
	})
}

func TestSyncSnapshots(t *testing.T) {
	for _, scenario := range []string{"aligned", "behind", "dirty", "ahead", "diverged", "unavailable", "wrong-branch", "main-dirty", "main-branch"} {
		t.Run(scenario, func(t *testing.T) {
			main, target, branch := newSyncRepository(t)
			worktree := fixedWorktree{branch: branch, path: target}
			switch scenario {
			case "behind":
				commitTest(t, main, "main.txt", "main")
			case "dirty":
				mustWrite(t, filepath.Join(target, "dirty.txt"), "dirty")
			case "ahead":
				commitTest(t, target, "branch.txt", "branch")
			case "diverged":
				commitTest(t, target, "branch.txt", "branch")
				commitTest(t, main, "main.txt", "main")
			case "unavailable":
				worktree.path = filepath.Join(filepath.Dir(target), "missing")
			case "wrong-branch":
				gitTest(t, main, "branch", "codex/other")
				worktree.branch = "codex/other"
			case "main-dirty":
				mustWrite(t, filepath.Join(main, "dirty.txt"), "dirty")
			case "main-branch":
				gitTest(t, main, "switch", "-c", "other")
			}
			snapshot := inspectSyncSnapshot(main, []fixedWorktree{worktree})
			candidate := snapshot.candidates[0]
			switch scenario {
			case "aligned":
				if candidate.state != syncAligned {
					t.Fatalf("got %+v", candidate)
				}
			case "behind":
				if candidate.state != syncSafe {
					t.Fatalf("got %+v", candidate)
				}
			case "dirty":
				if candidate.reason != "worktree has uncommitted changes" || snapshot.dirtyWorktrees != 1 {
					t.Fatalf("got %+v snapshot=%+v", candidate, snapshot)
				}
			case "ahead", "diverged":
				if candidate.state != syncUnsafe || !strings.Contains(candidate.reason, scenario) {
					t.Fatalf("got %+v", candidate)
				}
			case "unavailable":
				if candidate.reason != "fixed worktree is unavailable" || snapshot.unavailablePaths != 1 {
					t.Fatalf("got %+v snapshot=%+v", candidate, snapshot)
				}
			case "wrong-branch":
				if !strings.Contains(candidate.reason, "expected branch") {
					t.Fatalf("got %+v", candidate)
				}
			case "main-dirty":
				if snapshot.mainIssue != "" || snapshot.dirtyWorktrees != 1 {
					t.Fatalf("got %+v", snapshot)
				}
			case "main-branch":
				if !strings.Contains(snapshot.mainIssue, "expected branch main") {
					t.Fatalf("got %+v", snapshot)
				}
			}
		})
	}
}

func TestSyncCommandDispatch(t *testing.T) {
	t.Run("explicit branch fast-forwards through dispatch", func(t *testing.T) {
		main, target, branch := newSyncRepository(t)
		commitTest(t, main, "main.txt", "main")
		a := &app{
			root:          main,
			in:            strings.NewReader(""),
			out:           io.Discard,
			errOut:        io.Discard,
			syncMainPath:  main,
			syncWorktrees: []fixedWorktree{{branch: branch, path: target}},
		}
		if err := a.dispatch([]string{"sync", "--branch", branch}); err != nil {
			t.Fatal(err)
		}
		if gitTest(t, target, "rev-parse", "HEAD") != gitTest(t, main, "rev-parse", "HEAD") {
			t.Fatal("dispatch did not fast-forward the selected branch")
		}
	})

	t.Run("non-terminal dispatch requires explicit selection", func(t *testing.T) {
		main, target, branch := newSyncRepository(t)
		commitTest(t, main, "main.txt", "main")
		before := gitTest(t, target, "rev-parse", "HEAD")
		a := &app{
			root:          main,
			in:            strings.NewReader("1\n"),
			out:           io.Discard,
			errOut:        io.Discard,
			syncMainPath:  main,
			syncWorktrees: []fixedWorktree{{branch: branch, path: target}},
		}
		if err := a.dispatch([]string{"sync"}); err == nil || !strings.Contains(err.Error(), "requires --all or --branch") {
			t.Fatalf("unexpected error: %v", err)
		}
		if gitTest(t, target, "rev-parse", "HEAD") != before {
			t.Fatal("non-terminal dispatch changed the target")
		}
	})

	t.Run("flags reject conflict and unknown branch", func(t *testing.T) {
		a := &app{in: strings.NewReader(""), out: io.Discard, errOut: io.Discard}
		if err := a.dispatch([]string{"sync", "--all", "--branch", "codex/ui"}); err == nil || !strings.Contains(err.Error(), "mutually exclusive") {
			t.Fatalf("unexpected conflict error: %v", err)
		}
		if err := a.dispatch([]string{"sync", "--branch", "codex/unknown"}); err == nil || !strings.Contains(err.Error(), "not in the fixed AGENTS.md worktree table") {
			t.Fatalf("unexpected unknown-branch error: %v", err)
		}
	})
}

func TestSyncSelectionModes(t *testing.T) {
	t.Run("non-terminal refuses", func(t *testing.T) {
		main, target, branch := newSyncRepository(t)
		commitTest(t, main, "main.txt", "main")
		before := gitTest(t, target, "rev-parse", "HEAD")
		var out bytes.Buffer
		a := &app{in: strings.NewReader("a\n"), out: &out, errOut: io.Discard}
		err := a.syncFixedWorktrees(main, []fixedWorktree{{branch: branch, path: target}}, "", false, false)
		if err == nil || strings.Contains(out.String(), "Choice:") || gitTest(t, target, "rev-parse", "HEAD") != before {
			t.Fatalf("err=%v output=%s", err, out.String())
		}
	})

	t.Run("interactive quit and branch", func(t *testing.T) {
		main, target, branch := newSyncRepository(t)
		commitTest(t, main, "main.txt", "main")
		before := gitTest(t, target, "rev-parse", "HEAD")
		a := &app{in: strings.NewReader("q\n"), out: io.Discard, errOut: io.Discard}
		if err := a.syncFixedWorktrees(main, []fixedWorktree{{branch: branch, path: target}}, "", false, true); err != nil {
			t.Fatal(err)
		}
		if gitTest(t, target, "rev-parse", "HEAD") != before {
			t.Fatal("quit changed target")
		}
		a.in = strings.NewReader("1\n")
		if err := a.syncFixedWorktrees(main, []fixedWorktree{{branch: branch, path: target}}, "", false, true); err != nil {
			t.Fatal(err)
		}
		if gitTest(t, target, "rev-parse", "HEAD") != gitTest(t, main, "rev-parse", "HEAD") {
			t.Fatal("interactive branch did not fast-forward")
		}
	})

	t.Run("explicit all skips unsafe", func(t *testing.T) {
		main, target, branch := newSyncRepository(t)
		second := filepath.Join(filepath.Dir(target), "second")
		gitTest(t, main, "worktree", "add", "-b", "codex/second", second)
		commitTest(t, main, "main.txt", "main")
		mustWrite(t, filepath.Join(second, "dirty.txt"), "dirty")
		var out bytes.Buffer
		a := &app{in: strings.NewReader(""), out: &out, errOut: io.Discard}
		err := a.syncFixedWorktrees(main, []fixedWorktree{{branch: branch, path: target}, {branch: "codex/second", path: second}}, "", true, false)
		if err == nil || !strings.Contains(out.String(), "[ok] "+branch) || !strings.Contains(out.String(), "[skip] codex/second") {
			t.Fatalf("err=%v output=%s", err, out.String())
		}
		if gitTest(t, target, "rev-parse", "HEAD") != gitTest(t, main, "rev-parse", "HEAD") {
			t.Fatal("safe target did not fast-forward")
		}
	})

	t.Run("interactive all skips unsafe", func(t *testing.T) {
		main, target, branch := newSyncRepository(t)
		second := filepath.Join(filepath.Dir(target), "second")
		gitTest(t, main, "worktree", "add", "-b", "codex/second", second)
		commitTest(t, main, "main.txt", "main")
		mustWrite(t, filepath.Join(main, "main-dirty.txt"), "dirty")
		mustWrite(t, filepath.Join(second, "dirty.txt"), "dirty")
		var out bytes.Buffer
		a := &app{in: strings.NewReader("A\n"), out: &out, errOut: io.Discard}
		if err := a.syncFixedWorktrees(main, []fixedWorktree{{branch: branch, path: target}, {branch: "codex/second", path: second}}, "", false, true); err != nil {
			t.Fatal(err)
		}
		if gitTest(t, target, "rev-parse", "HEAD") != gitTest(t, main, "rev-parse", "HEAD") {
			t.Fatal("interactive all did not fast-forward safe target")
		}
		text := out.String()
		latest := strings.Split(text, "Latest status:")
		if !strings.Contains(text, "[skip] codex/second") || len(latest) != 2 || !strings.Contains(latest[1], "Fixed branch status:") || !strings.Contains(latest[1], branch) || !strings.Contains(latest[1], "[aligned]") {
			t.Fatalf("missing final status in:\n%s", text)
		}
	})

	t.Run("explicit branch", func(t *testing.T) {
		main, target, branch := newSyncRepository(t)
		commitTest(t, main, "main.txt", "main")
		a := &app{in: strings.NewReader(""), out: io.Discard, errOut: io.Discard}
		if err := a.syncFixedWorktrees(main, []fixedWorktree{{branch: branch, path: target}}, branch, false, false); err != nil {
			t.Fatal(err)
		}
	})
}

func newSyncRepository(t *testing.T) (string, string, string) {
	t.Helper()
	base := t.TempDir()
	main := filepath.Join(base, "main")
	target := filepath.Join(base, "target")
	branch := "codex/test"
	gitTest(t, base, "init", "-b", "main", main)
	gitTest(t, main, "config", "user.email", "echo@example.invalid")
	gitTest(t, main, "config", "user.name", "echo test")
	commitTest(t, main, "base.txt", "base")
	gitTest(t, main, "worktree", "add", "-b", branch, target)
	return main, target, branch
}

func commitTest(t *testing.T, repo, name, value string) {
	t.Helper()
	mustWrite(t, filepath.Join(repo, name), value)
	gitTest(t, repo, "add", name)
	gitTest(t, repo, "commit", "-m", value)
}

func mustWrite(t *testing.T, path, value string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(value), 0o644); err != nil {
		t.Fatal(err)
	}
}

func gitTest(t *testing.T, path string, args ...string) string {
	t.Helper()
	all := append([]string{"-C", path}, args...)
	cmd := exec.Command("git", all...)
	cmd.Env = append(os.Environ(), "GIT_CONFIG_NOSYSTEM=1", "GIT_TERMINAL_PROMPT=0")
	output, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git %s: %v\n%s", strings.Join(args, " "), err, output)
	}
	return strings.TrimSpace(string(output))
}
