//go:build windows

package main

import (
	"encoding/base64"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"time"
	"unsafe"
)

const (
	wsOverlappedWindow = 0x00CF0000
	wsChild            = 0x40000000
	wsVisible          = 0x10000000
	wsBorder           = 0x00800000
	wsTabStop          = 0x00010000
	wsExClientEdge     = 0x00000200
	esAutoHScroll      = 0x00000080
	esPassword         = 0x00000020
	esReadOnly         = 0x00000800
	swShow             = 5
	wmDestroy          = 0x0002
	wmClose            = 0x0010
	wmActivate         = 0x0006
	wmSetFocus         = 0x0007
	wmCommand          = 0x0111
	wmTimer            = 0x0113
	emSetSel           = 0x00B1
	wmUser             = 0x0400
	targetTimerID      = 1
	commandMessage     = wmUser + 1

	colorWindow  = 5
	idcArrow     = 32512
	cwUseDefault = 0x80000000

	swpNoSize     = 0x0001
	swpNoMove     = 0x0002
	swpNoActivate = 0x0010
	swpShowWindow = 0x0040
	mouseLeftDown = 0x0002
	mouseLeftUp   = 0x0004
)

var (
	targetUser32            = syscall.NewLazyDLL("user32.dll")
	targetKernel32          = syscall.NewLazyDLL("kernel32.dll")
	procRegisterClassEx     = targetUser32.NewProc("RegisterClassExW")
	procCreateWindowEx      = targetUser32.NewProc("CreateWindowExW")
	procDefWindowProc       = targetUser32.NewProc("DefWindowProcW")
	procShowWindow          = targetUser32.NewProc("ShowWindow")
	procUpdateWindow        = targetUser32.NewProc("UpdateWindow")
	procSetTimer            = targetUser32.NewProc("SetTimer")
	procKillTimer           = targetUser32.NewProc("KillTimer")
	procGetMessage          = targetUser32.NewProc("GetMessageW")
	procTranslateMessage    = targetUser32.NewProc("TranslateMessage")
	procDispatchMessage     = targetUser32.NewProc("DispatchMessageW")
	procPostQuitMessage     = targetUser32.NewProc("PostQuitMessage")
	procDestroyWindow       = targetUser32.NewProc("DestroyWindow")
	procGetModuleHandle     = targetKernel32.NewProc("GetModuleHandleW")
	procGetCurrentThreadID  = targetKernel32.NewProc("GetCurrentThreadId")
	procGetForegroundWindow = targetUser32.NewProc("GetForegroundWindow")
	procGetWindowThread     = targetUser32.NewProc("GetWindowThreadProcessId")
	procSetForegroundWindow = targetUser32.NewProc("SetForegroundWindow")
	procSetActiveWindow     = targetUser32.NewProc("SetActiveWindow")
	procBringWindowToTop    = targetUser32.NewProc("BringWindowToTop")
	procSwitchToThisWindow  = targetUser32.NewProc("SwitchToThisWindow")
	procAttachThreadInput   = targetUser32.NewProc("AttachThreadInput")
	procAllowSetForeground  = targetUser32.NewProc("AllowSetForegroundWindow")
	procSetFocus            = targetUser32.NewProc("SetFocus")
	procGetFocus            = targetUser32.NewProc("GetFocus")
	procSendMessage         = targetUser32.NewProc("SendMessageW")
	procGetWindowTextLength = targetUser32.NewProc("GetWindowTextLengthW")
	procGetWindowText       = targetUser32.NewProc("GetWindowTextW")
	procLoadCursor          = targetUser32.NewProc("LoadCursorW")
	procSetWindowPos        = targetUser32.NewProc("SetWindowPos")
	procGetCursorPos        = targetUser32.NewProc("GetCursorPos")
	procSetCursorPos        = targetUser32.NewProc("SetCursorPos")
	procGetWindowRect       = targetUser32.NewProc("GetWindowRect")
	procMouseEvent          = targetUser32.NewProc("mouse_event")
)

type targetFlags struct {
	runID           string
	title           string
	readyPath       string
	commandPath     string
	responsePath    string
	primaryOutput   string
	secondaryOutput string
	passwordOutput  string
	readonlyOutput  string
	unknownOutput   string
}

type targetRequest struct {
	RunID     string `json:"run_id"`
	RequestID string `json:"request_id"`
	Command   string `json:"command"`
	Payload   string `json:"payload"`
}

type targetResponse struct {
	RequestID string `json:"request_id"`
	Value     string `json:"value"`
	Error     string `json:"error"`
}

type targetReady struct {
	RunID     string `json:"run_id"`
	ProcessID int    `json:"process_id"`
	Title     string `json:"title"`
}

type targetPoint struct {
	X int32
	Y int32
}

type targetRect struct {
	Left   int32
	Top    int32
	Right  int32
	Bottom int32
}

type targetMsg struct {
	HWND     uintptr
	Message  uint32
	_        uint32
	WParam   uintptr
	LParam   uintptr
	Time     uint32
	Point    targetPoint
	LPrivate uint32
}

type targetWndClassEx struct {
	Size        uint32
	Style       uint32
	WndProc     uintptr
	ClassExtra  int32
	WindowExtra int32
	Instance    uintptr
	Icon        uintptr
	Cursor      uintptr
	Background  uintptr
	MenuName    uintptr
	ClassName   uintptr
	SmallIcon   uintptr
}

type targetState struct {
	flags       targetFlags
	window      uintptr
	primary     uintptr
	secondary   uintptr
	password    uintptr
	readonly    uintptr
	unknown     uintptr
	lastFocused uintptr
	callback    uintptr
	className   *uint16
	timerActive bool
}

func runTarget(args []string) error {
	flags, err := parseTargetFlags(args)
	if err != nil {
		return err
	}
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()

	state := &targetState{flags: flags}
	return state.run()
}

func parseTargetFlags(args []string) (targetFlags, error) {
	fs := flag.NewFlagSet("target", flag.ContinueOnError)
	flags := targetFlags{}
	fs.StringVar(&flags.runID, "run-id", "", "fixture run identity")
	fs.StringVar(&flags.title, "title", "Echo native target fixture", "window title")
	fs.StringVar(&flags.readyPath, "ready", "", "ready file")
	fs.StringVar(&flags.commandPath, "command", "", "command file")
	fs.StringVar(&flags.responsePath, "response", "", "response file")
	fs.StringVar(&flags.primaryOutput, "primary", "", "primary output file")
	fs.StringVar(&flags.secondaryOutput, "secondary", "", "secondary output file")
	fs.StringVar(&flags.passwordOutput, "password", "", "password output file")
	fs.StringVar(&flags.readonlyOutput, "readonly", "", "read-only output file")
	fs.StringVar(&flags.unknownOutput, "unknown", "", "unknown target output file")
	if err := fs.Parse(args); err != nil {
		return targetFlags{}, err
	}
	if fs.NArg() != 0 {
		return targetFlags{}, fmt.Errorf("unexpected target fixture arguments: %s", strings.Join(fs.Args(), " "))
	}
	if flags.runID == "" {
		return targetFlags{}, fmt.Errorf("target fixture run id is required")
	}
	for name, path := range map[string]string{
		"ready":     flags.readyPath,
		"command":   flags.commandPath,
		"response":  flags.responsePath,
		"primary":   flags.primaryOutput,
		"secondary": flags.secondaryOutput,
		"password":  flags.passwordOutput,
		"readonly":  flags.readonlyOutput,
		"unknown":   flags.unknownOutput,
	} {
		if path == "" || !filepath.IsAbs(path) {
			return targetFlags{}, fmt.Errorf("target fixture %s path must be absolute", name)
		}
	}
	return flags, nil
}

func (state *targetState) run() error {
	if err := state.createWindow(); err != nil {
		return err
	}
	if err := state.writeOutputs(); err != nil {
		return err
	}
	if err := state.focusControl(state.primary, 1); err != nil {
		return err
	}
	ready, err := json.Marshal(targetReady{
		RunID:     state.flags.runID,
		ProcessID: os.Getpid(),
		Title:     state.flags.title,
	})
	if err != nil {
		return err
	}
	if err := writeAtomic(state.flags.readyPath, ready); err != nil {
		return fmt.Errorf("write target fixture readiness: %w", err)
	}

	message := targetMsg{}
	for {
		result, _, callErr := procGetMessage.Call(uintptr(unsafe.Pointer(&message)), 0, 0, 0)
		if int32(result) == -1 {
			return fmt.Errorf("GetMessage failed: %w", callErr)
		}
		if result == 0 {
			return nil
		}
		procTranslateMessage.Call(uintptr(unsafe.Pointer(&message)))
		procDispatchMessage.Call(uintptr(unsafe.Pointer(&message)))
	}
}

func (state *targetState) createWindow() error {
	state.className = mustUTF16("EchoNativeAcceptanceFixture")
	state.callback = syscall.NewCallback(func(hwnd, message, wParam, lParam uintptr) uintptr {
		return state.windowProc(hwnd, uint32(message), wParam, lParam)
	})
	instance, _, err := procGetModuleHandle.Call(0)
	if instance == 0 {
		return fmt.Errorf("GetModuleHandle failed: %w", err)
	}
	cursor, _, _ := procLoadCursor.Call(0, idcArrow)
	class := targetWndClassEx{
		Size:       uint32(unsafe.Sizeof(targetWndClassEx{})),
		Style:      0x0002 | 0x0001,
		WndProc:    state.callback,
		Instance:   instance,
		Cursor:     cursor,
		Background: colorWindow + 1,
		ClassName:  uintptr(unsafe.Pointer(state.className)),
	}
	if atom, _, callErr := procRegisterClassEx.Call(uintptr(unsafe.Pointer(&class))); atom == 0 {
		return fmt.Errorf("RegisterClassEx failed: %w", callErr)
	}

	window, _, callErr := procCreateWindowEx.Call(
		0,
		uintptr(unsafe.Pointer(state.className)),
		uintptr(unsafe.Pointer(mustUTF16(state.flags.title))),
		wsOverlappedWindow,
		cwUseDefault,
		cwUseDefault,
		520,
		360,
		0,
		0,
		instance,
		0,
	)
	if window == 0 {
		return fmt.Errorf("CreateWindowEx parent failed: %w", callErr)
	}
	state.window = window

	editClass := mustUTF16("EDIT")
	state.primary = state.createEdit(editClass, "ac", 101, 16, 18)
	state.secondary = state.createEdit(editClass, "", 102, 16, 78)
	state.password = state.createEdit(editClass, "", 103, 16, 138)
	state.readonly = state.createEdit(editClass, "readonly", 104, 16, 198)
	state.unknown = state.createUnknown(105, 16, 258)
	if state.primary == 0 || state.secondary == 0 || state.password == 0 || state.readonly == 0 || state.unknown == 0 {
		return fmt.Errorf("CreateWindowEx target control failed")
	}
	if result, _, callErr := procSetTimer.Call(state.window, targetTimerID, 25, 0); result == 0 {
		return fmt.Errorf("SetTimer failed: %w", callErr)
	}
	state.timerActive = true
	procShowWindow.Call(state.window, swShow)
	procUpdateWindow.Call(state.window)
	return nil
}

func (state *targetState) createEdit(className *uint16, value string, id, x, y int) uintptr {
	style := uintptr(wsChild | wsVisible | wsBorder | wsTabStop | esAutoHScroll)
	if id == 103 {
		style |= esPassword
	}
	if id == 104 {
		style |= esReadOnly
	}
	window, _, _ := procCreateWindowEx.Call(
		wsExClientEdge,
		uintptr(unsafe.Pointer(className)),
		uintptr(unsafe.Pointer(mustUTF16(value))),
		style,
		uintptr(x),
		uintptr(y),
		450,
		38,
		state.window,
		uintptr(id),
		0,
		0,
	)
	return window
}

func (state *targetState) createUnknown(id, x, y int) uintptr {
	className := mustUTF16("BUTTON")
	style := uintptr(wsChild | wsVisible | wsBorder | wsTabStop)
	returnWindow, _, _ := procCreateWindowEx.Call(
		0,
		uintptr(unsafe.Pointer(className)),
		uintptr(unsafe.Pointer(mustUTF16("unknown unsafe target"))),
		style,
		uintptr(x),
		uintptr(y),
		450,
		38,
		state.window,
		uintptr(id),
		0,
		0,
	)
	return returnWindow
}

func (state *targetState) windowProc(hwnd uintptr, message uint32, wParam, lParam uintptr) uintptr {
	switch message {
	case wmTimer:
		if wParam == targetTimerID {
			_ = state.handleCommand()
		}
	case wmClose:
		procDestroyWindow.Call(hwnd)
	case wmActivate:
		result, _, _ := procDefWindowProc.Call(hwnd, uintptr(message), wParam, lParam)
		if wParam != 0 && state.lastFocused != 0 {
			procSetFocus.Call(state.lastFocused)
		}
		return result
	case wmSetFocus:
		result, _, _ := procDefWindowProc.Call(hwnd, uintptr(message), wParam, lParam)
		if state.lastFocused != 0 {
			procSetFocus.Call(state.lastFocused)
		}
		return result
	case wmDestroy:
		if state.timerActive {
			procKillTimer.Call(hwnd, targetTimerID)
			state.timerActive = false
		}
		procPostQuitMessage.Call(0)
	case wmCommand, commandMessage:
		// The controls do not require command handling. This case deliberately
		// leaves the native edit controls owned by the Win32 message loop.
	}
	result, _, _ := procDefWindowProc.Call(hwnd, uintptr(message), wParam, lParam)
	return result
}

func (state *targetState) handleCommand() error {
	command, err := os.ReadFile(state.flags.commandPath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}
	_ = os.Remove(state.flags.commandPath)

	request := targetRequest{}
	value := ""
	errorMessage := ""
	if err := json.Unmarshal(command, &request); err != nil {
		errorMessage = err.Error()
	} else if request.RunID != state.flags.runID || request.RequestID == "" {
		errorMessage = "Echo target fixture command ownership is invalid"
	} else {
		value, errorMessage = state.executeCommand(request.Command, request.Payload)
	}
	response, err := json.Marshal(targetResponse{
		RequestID: request.RequestID,
		Value:     base64.StdEncoding.EncodeToString([]byte(value)),
		Error:     base64.StdEncoding.EncodeToString([]byte(errorMessage)),
	})
	if err != nil {
		return err
	}
	if err := writeAtomic(state.flags.responsePath, response); err != nil {
		return err
	}
	return nil
}

func (state *targetState) executeCommand(command, payload string) (string, string) {
	switch command {
	case "allow-foreground":
		processID, err := strconv.ParseUint(payload, 10, 32)
		if err != nil || processID == 0 {
			return "", "Echo process id is invalid"
		}
		result, _, callErr := procAllowSetForeground.Call(uintptr(processID))
		if result == 0 {
			return "", fmt.Sprintf("AllowSetForegroundWindow failed: %v", callErr)
		}
		return "true", ""
	case "focus-primary":
		return "", state.commandError(state.focusControl(state.primary, 1))
	case "focus-secondary":
		return "", state.commandError(state.focusControl(state.secondary, 0))
	case "focus-password":
		return "", state.commandError(state.focusControl(state.password, -1))
	case "focus-readonly":
		return "", state.commandError(state.focusControl(state.readonly, 0))
	case "focus-unknown":
		return "", state.commandError(state.focusControl(state.unknown, 0))
	case "primary-focused":
		foreground, _, _ := procGetForegroundWindow.Call()
		focus, _, _ := procGetFocus.Call()
		return strconv.FormatBool(foreground == state.window && focus == state.primary), ""
	case "read-primary":
		return state.windowText(state.primary), ""
	case "read-secondary":
		return state.windowText(state.secondary), ""
	case "read-password":
		return state.windowText(state.password), ""
	case "read-readonly":
		return state.windowText(state.readonly), ""
	case "read-unknown":
		return state.windowText(state.unknown), ""
	case "copy-password":
		if err := state.focusControl(state.password, -1); err != nil {
			return "", err.Error()
		}
		if err := writeClipboard("copy-text", "password-secret"); err != nil {
			return "", err.Error()
		}
		return "", ""
	case "shutdown":
		procDestroyWindow.Call(state.window)
		return "", ""
	default:
		return "", fmt.Sprintf("Unknown Echo target fixture command %q", command)
	}
}

func (state *targetState) commandError(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}

func (state *targetState) focusControl(control uintptr, caret int) error {
	if control == 0 || !state.activateWindow() {
		return fmt.Errorf("Echo target fixture could not become foreground")
	}
	procSetFocus.Call(control)
	if caret >= 0 {
		procSendMessage.Call(control, emSetSel, uintptr(caret), uintptr(caret))
	}
	state.lastFocused = control
	if focus, _, _ := procGetFocus.Call(); focus != control {
		return fmt.Errorf("Echo target fixture focus did not settle")
	}
	return nil
}

func (state *targetState) activateWindow() bool {
	for attempt := 0; attempt < 20; attempt++ {
		procShowWindow.Call(state.window, swShow)
		procSetActiveWindow.Call(state.window)
		procBringWindowToTop.Call(state.window)
		procSetForegroundWindow.Call(state.window)
		if foreground, _, _ := procGetForegroundWindow.Call(); foreground == state.window {
			return true
		}

		procSwitchToThisWindow.Call(state.window, 0)
		foreground, _, _ := procGetForegroundWindow.Call()
		foregroundThread := windowThread(foreground)
		currentThread, _, _ := procGetCurrentThreadID.Call()
		attached := foregroundThread != 0 && foregroundThread != currentThread
		if attached {
			procAttachThreadInput.Call(currentThread, foregroundThread, 1)
		}
		procSetActiveWindow.Call(state.window)
		procBringWindowToTop.Call(state.window)
		procSetForegroundWindow.Call(state.window)
		if attached {
			procAttachThreadInput.Call(currentThread, foregroundThread, 0)
		}
		if foreground, _, _ := procGetForegroundWindow.Call(); foreground == state.window {
			return true
		}
		if state.mouseActivate() {
			return true
		}
		time.Sleep(25 * time.Millisecond)
	}
	return false
}

func (state *targetState) mouseActivate() bool {
	var original, bounds targetPointOrRect
	if !getCursorPos(&original.point) || !getWindowRect(state.window, &bounds.rect) {
		return false
	}
	procSetWindowPos.Call(state.window, ^uintptr(0), 0, 0, 0, 0, swpNoSize|swpNoMove|swpNoActivate|swpShowWindow)
	centerX := bounds.rect.Left + (bounds.rect.Right-bounds.rect.Left)/2
	centerY := bounds.rect.Top + (bounds.rect.Bottom-bounds.rect.Top)/2
	procSetCursorPos.Call(uintptr(centerX), uintptr(centerY))
	procMouseEvent.Call(mouseLeftDown, 0, 0, 0, 0)
	procMouseEvent.Call(mouseLeftUp, 0, 0, 0, 0)
	procSetWindowPos.Call(state.window, ^uintptr(1), 0, 0, 0, 0, swpNoSize|swpNoMove|swpNoActivate|swpShowWindow)
	procSetCursorPos.Call(uintptr(original.point.X), uintptr(original.point.Y))
	foreground, _, _ := procGetForegroundWindow.Call()
	return foreground == state.window
}

type targetPointOrRect struct {
	point targetPoint
	rect  targetRect
}

func windowThread(window uintptr) uintptr {
	if window == 0 {
		return 0
	}
	thread, _, _ := procGetWindowThread.Call(window, 0)
	return thread
}

func getCursorPos(point *targetPoint) bool {
	result, _, _ := procGetCursorPos.Call(uintptr(unsafe.Pointer(point)))
	return result != 0
}

func getWindowRect(window uintptr, rect *targetRect) bool {
	result, _, _ := procGetWindowRect.Call(window, uintptr(unsafe.Pointer(rect)))
	return result != 0
}

func (state *targetState) windowText(window uintptr) string {
	length, _, _ := procGetWindowTextLength.Call(window)
	values := make([]uint16, int(length)+1)
	if len(values) == 0 {
		return ""
	}
	procGetWindowText.Call(window, uintptr(unsafe.Pointer(&values[0])), uintptr(len(values)))
	for index, value := range values {
		if value == 0 {
			values = values[:index]
			break
		}
	}
	return stringFromUTF16(values)
}

func (state *targetState) writeOutputs() error {
	outputs := map[string]string{
		state.flags.primaryOutput:   state.windowText(state.primary),
		state.flags.secondaryOutput: state.windowText(state.secondary),
		state.flags.passwordOutput:  state.windowText(state.password),
		state.flags.readonlyOutput:  state.windowText(state.readonly),
		state.flags.unknownOutput:   state.windowText(state.unknown),
	}
	for path, value := range outputs {
		if err := writeAtomic(path, []byte(value)); err != nil {
			return err
		}
	}
	return nil
}

func writeAtomic(path string, data []byte) error {
	temporary := path + ".tmp"
	if err := os.WriteFile(temporary, data, 0o644); err != nil {
		return err
	}
	for attempt := 0; attempt < 20; attempt++ {
		_ = os.Remove(path)
		if err := os.Rename(temporary, path); err == nil {
			return nil
		}
		time.Sleep(10 * time.Millisecond)
	}
	_ = os.Remove(temporary)
	return fmt.Errorf("atomic replace failed for %s", path)
}

func mustUTF16(value string) *uint16 {
	pointer, err := syscall.UTF16PtrFromString(value)
	if err != nil {
		panic(err)
	}
	return pointer
}

func stringFromUTF16(values []uint16) string {
	return string(syscall.UTF16ToString(values))
}
