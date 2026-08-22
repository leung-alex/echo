//go:build windows

package main

import (
	"encoding/binary"
	"flag"
	"fmt"
	"os"
	"runtime"
	"strings"
	"syscall"
	"time"
	"unicode/utf16"
	"unsafe"
)

const (
	cfDIB          = 8
	cfUnicodeText  = 13
	cfHDROP        = 15
	gmemMoveable   = 0x0002
	bitmapWidth    = 24
	bitmapHeight   = 24
	bitmapRowBytes = bitmapWidth * 4
)

var (
	clipboardUser32       = syscall.NewLazyDLL("user32.dll")
	clipboardKernel32     = syscall.NewLazyDLL("kernel32.dll")
	procOpenClipboard     = clipboardUser32.NewProc("OpenClipboard")
	procCloseClipboard    = clipboardUser32.NewProc("CloseClipboard")
	procEmptyClipboard    = clipboardUser32.NewProc("EmptyClipboard")
	procSetClipboardData  = clipboardUser32.NewProc("SetClipboardData")
	procGetClipboardData  = clipboardUser32.NewProc("GetClipboardData")
	procRegisterClipboard = clipboardUser32.NewProc("RegisterClipboardFormatW")
	procGlobalAlloc       = clipboardKernel32.NewProc("GlobalAlloc")
	procGlobalLock        = clipboardKernel32.NewProc("GlobalLock")
	procGlobalUnlock      = clipboardKernel32.NewProc("GlobalUnlock")
	procGlobalFree        = clipboardKernel32.NewProc("GlobalFree")
	procGlobalSize        = clipboardKernel32.NewProc("GlobalSize")
	procRtlMoveMemory     = clipboardKernel32.NewProc("RtlMoveMemory")
)

func main() {
	if len(os.Args) < 2 {
		fatalf("usage: echo-native-fixture <clipboard|target|target-elevated> ...")
	}
	var err error
	switch os.Args[1] {
	case "clipboard":
		err = runClipboard(os.Args[2:])
	case "target":
		err = runTarget(os.Args[2:])
	case "target-elevated":
		err = runTargetElevated(os.Args[2:])
	default:
		err = fmt.Errorf("unknown fixture mode %q", os.Args[1])
	}
	if err != nil {
		fatalf("%v", err)
	}
}

func fatalf(format string, args ...any) {
	_, _ = fmt.Fprintf(os.Stderr, format+"\n", args...)
	os.Exit(1)
}

func runClipboard(args []string) error {
	fs := flag.NewFlagSet("clipboard", flag.ContinueOnError)
	operation := fs.String("operation", "", "clipboard fixture operation")
	value := fs.String("value", "fixture", "fixture value")
	if err := fs.Parse(args); err != nil {
		return err
	}
	if fs.NArg() != 0 {
		return fmt.Errorf("unexpected clipboard fixture arguments: %s", strings.Join(fs.Args(), " "))
	}

	if *operation == "read-text" {
		text, err := readClipboardText()
		if err != nil {
			return err
		}
		_, err = fmt.Fprintln(os.Stdout, text)
		return err
	}
	if *operation == "hold-open" {
		return holdClipboardOpen()
	}

	return writeClipboard(*operation, *value)
}

func holdClipboardOpen() error {
	runtime.LockOSThread()
	className := mustUTF16("STATIC")
	title := mustUTF16("Echo clipboard holder")
	owner, _, callErr := procCreateWindowEx.Call(
		0,
		uintptr(unsafe.Pointer(className)),
		uintptr(unsafe.Pointer(title)),
		wsOverlappedWindow,
		cwUseDefault,
		cwUseDefault,
		1,
		1,
		0,
		0,
		0,
		0,
	)
	if owner == 0 {
		runtime.UnlockOSThread()
		return fmt.Errorf("CreateWindowEx clipboard owner failed: %w", callErr)
	}
	defer procDestroyWindow.Call(owner)
	procShowWindow.Call(owner, 0)
	if result, _, callErr := procOpenClipboard.Call(owner); result == 0 {
		runtime.UnlockOSThread()
		return fmt.Errorf("OpenClipboard failed: %w", callErr)
	}
	fmt.Fprintln(os.Stdout, "ready")
	for {
		time.Sleep(time.Hour)
	}
}

func writeClipboard(operation, value string) error {
	var filesPath string
	items, err := clipboardItems(operation, value)
	if err != nil {
		return err
	}
	if operation == "copy-files" {
		filesPath = string(items[len(items)-1].data)
		items = items[:len(items)-1]
	}
	if err := withClipboard(func() error {
		if result, _, callErr := procEmptyClipboard.Call(); result == 0 {
			return fmt.Errorf("EmptyClipboard failed: %w", callErr)
		}
		for _, item := range items {
			if err := setClipboardData(item.format, item.data); err != nil {
				return err
			}
		}
		return nil
	}); err != nil {
		return err
	}
	if filesPath != "" {
		_, err = fmt.Fprintln(os.Stdout, filesPath)
	}
	return err
}

type clipboardItem struct {
	format uint32
	data   []byte
}

func clipboardItems(operation, value string) ([]clipboardItem, error) {
	unicodeText := clipboardItem{format: cfUnicodeText, data: utf16Bytes(value)}
	switch operation {
	case "copy-text":
		return []clipboardItem{unicodeText}, nil
	case "copy-html":
		htmlFormat, err := registerClipboardFormat("HTML Format")
		if err != nil {
			return nil, err
		}
		return []clipboardItem{unicodeText, {format: htmlFormat, data: htmlPayload(value)}}, nil
	case "copy-rtf":
		rtfFormat, err := registerClipboardFormat("Rich Text Format")
		if err != nil {
			return nil, err
		}
		rtf := fmt.Sprintf(`{\rtf1\ansi\b %s\b0}`, value)
		return []clipboardItem{unicodeText, {format: rtfFormat, data: []byte(rtf)}}, nil
	case "copy-image":
		return []clipboardItem{{format: cfDIB, data: dibPayload()}}, nil
	case "copy-files":
		path, err := createClipboardFixtureFile(value)
		if err != nil {
			return nil, err
		}
		return []clipboardItem{{format: cfHDROP, data: dropFilesPayload(path)}, {data: []byte(path)}}, nil
	case "copy-unsupported":
		format, err := registerClipboardFormat("Echo.Unsupported")
		if err != nil {
			return nil, err
		}
		return []clipboardItem{{format: format, data: []byte(value)}}, nil
	default:
		return nil, fmt.Errorf("unknown clipboard fixture operation %q", operation)
	}
}

func withClipboard(fn func() error) error {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()

	var lastErr error
	for attempt := 0; attempt < 40; attempt++ {
		if result, _, err := procOpenClipboard.Call(0); result != 0 {
			defer procCloseClipboard.Call()
			return fn()
		} else {
			lastErr = err
		}
		time.Sleep(25 * time.Millisecond)
	}
	return fmt.Errorf("OpenClipboard failed after retries: %w", lastErr)
}

func readClipboardText() (string, error) {
	var result string
	err := withClipboard(func() error {
		handle, _, err := procGetClipboardData.Call(cfUnicodeText)
		if handle == 0 {
			return fmt.Errorf("GetClipboardData(CF_UNICODETEXT) failed: %w", err)
		}
		pointer, _, err := procGlobalLock.Call(handle)
		if pointer == 0 {
			return fmt.Errorf("GlobalLock failed: %w", err)
		}
		defer procGlobalUnlock.Call(handle)

		size, _, _ := procGlobalSize.Call(handle)
		words := int(size / 2)
		if words == 0 {
			result = ""
			return nil
		}
		bytes := make([]byte, words*2)
		procRtlMoveMemory.Call(uintptr(unsafe.Pointer(&bytes[0])), pointer, uintptr(len(bytes)))
		values := make([]uint16, words)
		for index := range values {
			values[index] = binary.LittleEndian.Uint16(bytes[index*2:])
		}
		for index, value := range values {
			if value == 0 {
				values = values[:index]
				break
			}
		}
		result = string(utf16.Decode(values))
		return nil
	})
	return result, err
}

func setClipboardData(format uint32, data []byte) error {
	handle, err := allocateGlobal(data)
	if err != nil {
		return err
	}
	if result, _, callErr := procSetClipboardData.Call(uintptr(format), handle); result == 0 {
		procGlobalFree.Call(handle)
		return fmt.Errorf("SetClipboardData(%d) failed: %w", format, callErr)
	}
	return nil
}

func allocateGlobal(data []byte) (uintptr, error) {
	handle, _, err := procGlobalAlloc.Call(gmemMoveable, uintptr(len(data)))
	if handle == 0 {
		return 0, fmt.Errorf("GlobalAlloc failed: %w", err)
	}
	pointer, _, err := procGlobalLock.Call(handle)
	if pointer == 0 {
		procGlobalFree.Call(handle)
		return 0, fmt.Errorf("GlobalLock failed: %w", err)
	}
	procRtlMoveMemory.Call(pointer, uintptr(unsafe.Pointer(&data[0])), uintptr(len(data)))
	procGlobalUnlock.Call(handle)
	return handle, nil
}

func registerClipboardFormat(name string) (uint32, error) {
	value := utf16Bytes(name)
	format, _, err := procRegisterClipboard.Call(uintptr(unsafe.Pointer(&value[0])))
	if format == 0 {
		return 0, fmt.Errorf("RegisterClipboardFormat(%q) failed: %w", name, err)
	}
	return uint32(format), nil
}

func createClipboardFixtureFile(value string) (string, error) {
	file, err := os.CreateTemp("", "echo-clipboard-*.txt")
	if err != nil {
		return "", fmt.Errorf("create clipboard fixture file: %w", err)
	}
	path := file.Name()
	if _, err := file.WriteString(value); err != nil {
		_ = file.Close()
		_ = os.Remove(path)
		return "", fmt.Errorf("write clipboard fixture file: %w", err)
	}
	if err := file.Close(); err != nil {
		_ = os.Remove(path)
		return "", fmt.Errorf("close clipboard fixture file: %w", err)
	}
	return path, nil
}

func utf16Bytes(value string) []byte {
	words := utf16.Encode([]rune(value))
	words = append(words, 0)
	result := make([]byte, len(words)*2)
	for index, word := range words {
		binary.LittleEndian.PutUint16(result[index*2:], word)
	}
	return result
}

func htmlPayload(value string) []byte {
	const markerStart = "<!--StartFragment-->"
	const markerEnd = "<!--EndFragment-->"
	body := "<html><body>" + markerStart + "<b>" + value + "</b>" + markerEnd + "</body></html>"
	headerTemplate := "Version:0.9\r\nStartHTML:%08d\r\nEndHTML:%08d\r\nStartFragment:%08d\r\nEndFragment:%08d\r\n"
	headerLength := len([]byte(fmt.Sprintf(headerTemplate, 0, 0, 0, 0)))
	bodyBytes := []byte(body)
	startHTML := headerLength
	endHTML := startHTML + len(bodyBytes)
	startMarker := strings.Index(body, markerStart)
	endMarker := strings.Index(body, markerEnd)
	startFragment := startHTML + len([]byte(body[:startMarker+len(markerStart)]))
	endFragment := startHTML + len([]byte(body[:endMarker]))
	header := fmt.Sprintf(headerTemplate, startHTML, endHTML, startFragment, endFragment)
	return append([]byte(header), bodyBytes...)
}

func dibPayload() []byte {
	dataSize := bitmapRowBytes * bitmapHeight
	result := make([]byte, 40+dataSize)
	binary.LittleEndian.PutUint32(result[0:], 40)
	binary.LittleEndian.PutUint32(result[4:], bitmapWidth)
	binary.LittleEndian.PutUint32(result[8:], bitmapHeight)
	binary.LittleEndian.PutUint16(result[12:], 1)
	binary.LittleEndian.PutUint16(result[14:], 32)
	binary.LittleEndian.PutUint32(result[20:], uint32(dataSize))
	for y := 0; y < bitmapHeight; y++ {
		for x := 0; x < bitmapWidth; x++ {
			red, green, blue := byte(100), byte(149), byte(237)
			if x >= 4 && x < 20 && y >= 4 && y < 20 {
				red, green, blue = 255, 215, 0
			}
			row := bitmapHeight - 1 - y
			offset := 40 + row*bitmapRowBytes + x*4
			result[offset] = blue
			result[offset+1] = green
			result[offset+2] = red
			result[offset+3] = 255
		}
	}
	return result
}

func dropFilesPayload(path string) []byte {
	paths := utf16Bytes(path + "\x00")
	result := make([]byte, 20+len(paths))
	binary.LittleEndian.PutUint32(result[0:], 20)
	binary.LittleEndian.PutUint32(result[16:], 1)
	copy(result[20:], paths)
	return result
}
