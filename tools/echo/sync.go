package main

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"os/exec"
	"sort"
	"strconv"
	"strings"
)

type fixedWorktree struct{ branch, path string }

const mainWorktreePath = `D:\Project\echo`

var fixedWorktrees = []fixedWorktree{
	{"codex/foundation", `D:\Worktrees\echo\foundation`},
	{"codex/ui", `D:\Worktrees\echo\ui`},
}

const (
	syncAligned = "aligned"
	syncSafe    = "safe"
	syncUnsafe  = "unsafe"
)

type syncCandidate struct {
	fixedWorktree
	head, state, reason string
}

type syncSnapshot struct {
	mainHead, mainIssue              string
	containsAll                      bool
	dirtyWorktrees, unavailablePaths int
	candidates                       []syncCandidate
}

func (a *app) syncCommand(args []string) error {
	fs := newFlags("sync", a.errOut)
	all := fs.Bool("all", false, "sync every fixed worktree")
	branch := fs.String("branch", "", "sync one fixed branch")
	if err := parseFlags(fs, args); err != nil {
		return err
	}
	if *all && *branch != "" {
		return fmt.Errorf("--all and --branch are mutually exclusive")
	}
	if *branch != "" && fixedWorktreeByBranch(a.syncWorktrees, *branch) == nil {
		return fmt.Errorf("branch %q is not in the fixed AGENTS.md worktree table", *branch)
	}
	return a.syncFixedWorktrees(a.syncMainPath, a.syncWorktrees, *branch, *all, isCharDevice(a.in))
}

func fixedWorktreeByBranch(worktrees []fixedWorktree, branch string) *fixedWorktree {
	for i := range worktrees {
		if worktrees[i].branch == branch {
			return &worktrees[i]
		}
	}
	return nil
}

func (a *app) syncFixedWorktrees(mainPath string, worktrees []fixedWorktree, branch string, all, interactive bool) error {
	snapshot := inspectSyncSnapshot(mainPath, worktrees)
	colors := colorsEnabled(a.out, a.env)
	renderSyncSummary(a.out, snapshot, colors)
	explicit := all || branch != ""
	if !explicit {
		renderSyncCandidates(a.out, snapshot.candidates, colors, "Select a branch to fast-forward to main:", interactive && snapshot.mainIssue == "")
	}
	if snapshot.mainIssue != "" {
		return fmt.Errorf("sync refused: %s", snapshot.mainIssue)
	}

	var selected []syncCandidate
	ignoreSkips := false
	if branch != "" {
		for _, candidate := range snapshot.candidates {
			if candidate.branch == branch {
				selected = []syncCandidate{candidate}
				break
			}
		}
	} else if all {
		selected = snapshot.candidates
	} else {
		if !interactive {
			return fmt.Errorf("sync requires --all or --branch when standard input is not a terminal")
		}
		choice, err := readSyncChoice(a.in, a.out, len(snapshot.candidates))
		if err != nil {
			return err
		}
		if choice == -1 {
			return nil
		}
		if choice == -2 {
			selected = snapshot.candidates
			ignoreSkips = true
		} else {
			selected = []syncCandidate{snapshot.candidates[choice]}
		}
	}
	err := a.syncCandidates(mainPath, snapshot.mainHead, selected, ignoreSkips)
	if !explicit {
		latest := inspectSyncSnapshot(mainPath, worktrees)
		fmt.Fprintln(a.out, "\nLatest status:")
		renderSyncSummary(a.out, latest, colors)
		renderSyncCandidates(a.out, latest.candidates, colors, "Fixed branch status:", false)
	}
	return err
}

func inspectSyncSnapshot(mainPath string, worktrees []fixedWorktree) syncSnapshot {
	snapshot := syncSnapshot{containsAll: true}
	worktrees = append([]fixedWorktree(nil), worktrees...)
	sort.Slice(worktrees, func(i, j int) bool { return worktrees[i].branch < worktrees[j].branch })

	if _, err := os.Stat(mainPath); err != nil {
		snapshot.mainIssue = "main worktree is unavailable: " + mainPath
	} else {
		snapshot.mainHead, _ = gitAt(mainPath, "rev-parse", "--verify", "refs/heads/main")
		current, err := gitAt(mainPath, "branch", "--show-current")
		if err != nil {
			snapshot.mainIssue = "main worktree is unavailable: " + mainPath
		} else if current != "main" {
			snapshot.mainIssue = fmt.Sprintf("main worktree expected branch main, found %s", current)
		}
		status, err := gitAt(mainPath, "status", "--porcelain")
		if err != nil && snapshot.mainIssue == "" {
			snapshot.mainIssue = "main worktree status is unavailable"
		} else if status != "" {
			snapshot.dirtyWorktrees++
		}
		if snapshot.mainHead == "" && snapshot.mainIssue == "" {
			snapshot.mainIssue = "main branch is unavailable"
		}
	}

	for _, worktree := range worktrees {
		candidate := syncCandidate{fixedWorktree: worktree, state: syncUnsafe}
		candidate.head, _ = gitAt(mainPath, "rev-parse", "--verify", "refs/heads/"+worktree.branch)
		_, pathErr := os.Stat(worktree.path)
		if pathErr != nil {
			snapshot.unavailablePaths++
		}
		if candidate.head == "" || snapshot.mainHead == "" || !gitOKAt(mainPath, "merge-base", "--is-ancestor", candidate.head, snapshot.mainHead) {
			snapshot.containsAll = false
		}
		if candidate.head == "" {
			candidate.reason = "fixed branch is unavailable"
		} else if pathErr != nil {
			candidate.reason = "fixed worktree is unavailable"
		} else {
			current, err := gitAt(worktree.path, "branch", "--show-current")
			if err != nil {
				candidate.reason = "fixed worktree is unavailable"
				snapshot.unavailablePaths++
			} else if current != worktree.branch {
				candidate.reason = fmt.Sprintf("expected branch %s, found %s", worktree.branch, current)
			} else if status, err := gitAt(worktree.path, "status", "--porcelain"); err != nil {
				candidate.reason = "worktree status is unavailable"
			} else if status != "" {
				candidate.reason = "worktree has uncommitted changes"
				snapshot.dirtyWorktrees++
			} else if candidate.head == snapshot.mainHead {
				candidate.state = syncAligned
			} else if gitOKAt(mainPath, "merge-base", "--is-ancestor", candidate.head, snapshot.mainHead) {
				candidate.state = syncSafe
			} else if gitOKAt(mainPath, "merge-base", "--is-ancestor", snapshot.mainHead, candidate.head) {
				candidate.reason = "branch is ahead of main"
			} else {
				candidate.reason = "branch has diverged from main"
			}
		}
		snapshot.candidates = append(snapshot.candidates, candidate)
	}
	return snapshot
}

func renderSyncSummary(out io.Writer, snapshot syncSnapshot, colors bool) {
	mainHash := shortHash(snapshot.mainHead)
	if snapshot.mainHead != "" {
		mainHash = colorize(mainHash, "36", colors)
	}
	contains := colorize("No", "31", colors)
	if snapshot.containsAll {
		contains = colorize("Yes", "32", colors)
	}
	fmt.Fprintf(out, "main: %s\n", mainHash)
	fmt.Fprintf(out, "main contains all fixed branch commits: %s\n", contains)
	fmt.Fprintf(out, "Worktrees with uncommitted changes: %s\n", coloredCount(snapshot.dirtyWorktrees, colors))
	fmt.Fprintf(out, "Unavailable fixed worktrees: %s\n", coloredCount(snapshot.unavailablePaths, colors))
}

func renderSyncCandidates(out io.Writer, candidates []syncCandidate, colors bool, title string, choices bool) {
	fmt.Fprintln(out, "\n"+title)
	width := 0
	for _, candidate := range candidates {
		if len(candidate.branch) > width {
			width = len(candidate.branch)
		}
	}
	for i, candidate := range candidates {
		hash := shortHash(candidate.head)
		if candidate.head != "" {
			hash = colorize(hash, "36", colors)
		}
		status := "[" + candidate.state + "]"
		if candidate.state == syncSafe {
			status += " fast-forward to main"
		} else if candidate.state == syncUnsafe {
			status += " " + candidate.reason
		}
		code := "32"
		if candidate.state == syncUnsafe {
			code = "31"
		}
		fmt.Fprintf(out, "[%d] %-*s %s %s\n", i+1, width, candidate.branch, hash, colorize(status, code, colors))
	}
	if choices {
		fmt.Fprintln(out, "[A] All fixed non-main branches (unsafe branches will be skipped)")
		fmt.Fprintln(out, "[Q] Quit")
	}
}

func readSyncChoice(in io.Reader, out io.Writer, count int) (int, error) {
	scanner := bufio.NewScanner(in)
	for {
		fmt.Fprint(out, "Choice: ")
		if !scanner.Scan() {
			if err := scanner.Err(); err != nil {
				return 0, fmt.Errorf("read sync choice: %w", err)
			}
			return 0, fmt.Errorf("sync choice input closed")
		}
		choice := strings.TrimSpace(scanner.Text())
		if strings.EqualFold(choice, "q") {
			return -1, nil
		}
		if strings.EqualFold(choice, "a") {
			return -2, nil
		}
		index, err := strconv.Atoi(choice)
		if err == nil && index >= 1 && index <= count {
			return index - 1, nil
		}
		fmt.Fprintf(out, "Invalid choice. Enter 1-%d, A, or Q.\n", count)
	}
}

func (a *app) syncCandidates(mainPath, mainHead string, candidates []syncCandidate, ignoreSkips bool) error {
	var failures []string
	for _, candidate := range candidates {
		if err := validateMainStable(mainPath, mainHead); err != nil {
			fmt.Fprintf(a.out, "[skip] %s: %v\n", candidate.branch, err)
			return err
		}
		if candidate.state == syncUnsafe {
			fmt.Fprintf(a.out, "[skip] %s: %s\n", candidate.branch, candidate.reason)
			failures = append(failures, candidate.branch)
			continue
		}
		result, err := a.syncOne(mainPath, mainHead, candidate.fixedWorktree)
		if err != nil {
			fmt.Fprintf(a.out, "[skip] %s: %v\n", candidate.branch, err)
			failures = append(failures, candidate.branch)
			continue
		}
		fmt.Fprintf(a.out, "[ok] %s: %s\n", candidate.branch, result)
	}
	if err := validateMainStable(mainPath, mainHead); err != nil {
		return err
	}
	if len(failures) > 0 && !ignoreSkips {
		return fmt.Errorf("sync skipped unsafe worktrees: %s", strings.Join(failures, ", "))
	}
	return nil
}

func (a *app) syncOne(mainPath, mainHead string, target fixedWorktree) (string, error) {
	if err := validateMainStable(mainPath, mainHead); err != nil {
		return "", err
	}
	if _, err := os.Stat(target.path); err != nil {
		return "", fmt.Errorf("fixed worktree is unavailable: %s", target.path)
	}
	if err := validateWorktree(target.path, target.branch); err != nil {
		return "", err
	}
	targetHead, err := gitAt(target.path, "rev-parse", "HEAD")
	if err != nil {
		return "", err
	}
	if targetHead == mainHead {
		return "aligned", nil
	}
	if gitOKAt(target.path, "merge-base", "--is-ancestor", targetHead, mainHead) {
		if _, err := gitAt(target.path, "merge", "--ff-only", mainHead); err != nil {
			return "", err
		}
		got, err := gitAt(target.path, "rev-parse", "HEAD")
		if err != nil {
			return "", err
		}
		if got != mainHead {
			return "", fmt.Errorf("fast-forward did not reach main head")
		}
		return "fast-forwarded to main", nil
	}
	if gitOKAt(target.path, "merge-base", "--is-ancestor", mainHead, targetHead) {
		return "", fmt.Errorf("branch is ahead of main")
	}
	return "", fmt.Errorf("branch has diverged from main")
}

func validateMainStable(mainPath, mainHead string) error {
	current, err := gitAt(mainPath, "rev-parse", "--verify", "refs/heads/main")
	if err != nil || current != mainHead {
		return fmt.Errorf("main changed during sync")
	}
	branch, err := gitAt(mainPath, "branch", "--show-current")
	if err != nil || branch != "main" {
		return fmt.Errorf("main changed during sync")
	}
	return nil
}

func validateWorktree(path, branch string) error {
	current, err := gitAt(path, "branch", "--show-current")
	if err != nil {
		return err
	}
	if current != branch {
		return fmt.Errorf("expected branch %s, found %s", branch, current)
	}
	status, err := gitAt(path, "status", "--porcelain")
	if err != nil {
		return err
	}
	if status != "" {
		return fmt.Errorf("worktree has uncommitted changes")
	}
	return nil
}

func shortHash(hash string) string {
	if hash == "" {
		return "------------"
	}
	if len(hash) > 12 {
		return hash[:12]
	}
	return hash
}

func coloredCount(value int, colors bool) string {
	code := "31"
	if value == 0 {
		code = "32"
	}
	return colorize(strconv.Itoa(value), code, colors)
}

func colorize(value, code string, enabled bool) string {
	if !enabled {
		return value
	}
	return "\x1b[" + code + "m" + value + "\x1b[0m"
}

func colorsEnabled(out io.Writer, env []string) bool {
	return isCharDevice(out) && colorsAllowed(env)
}

func colorsAllowed(env []string) bool {
	return !envDefined(env, "NO_COLOR") && !strings.EqualFold(envValue(env, "TERM"), "dumb")
}

func envDefined(env []string, name string) bool {
	prefix := strings.ToUpper(name) + "="
	for _, item := range env {
		if strings.HasPrefix(strings.ToUpper(item), prefix) {
			return true
		}
	}
	return false
}

func envValue(env []string, name string) string {
	prefix := strings.ToUpper(name) + "="
	for i := len(env) - 1; i >= 0; i-- {
		if strings.HasPrefix(strings.ToUpper(env[i]), prefix) {
			return env[i][len(prefix):]
		}
	}
	return ""
}

func isCharDevice(value any) bool {
	file, ok := value.(*os.File)
	if !ok {
		return false
	}
	info, err := file.Stat()
	return err == nil && info.Mode()&os.ModeCharDevice != 0
}

func gitAt(path string, args ...string) (string, error) {
	all := append([]string{"-C", path}, args...)
	cmd := exec.Command("git", all...)
	output, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("git %s: %w: %s", strings.Join(args, " "), err, strings.TrimSpace(string(output)))
	}
	return strings.TrimSpace(string(output)), nil
}

func gitOKAt(path string, args ...string) bool {
	all := append([]string{"-C", path}, args...)
	return exec.Command("git", all...).Run() == nil
}
