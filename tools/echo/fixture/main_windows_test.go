//go:build windows

package main

import (
	"encoding/binary"
	"fmt"
	"strings"
	"testing"
	"unicode/utf16"
)

func TestHTMLPayloadUsesFragmentOffsets(t *testing.T) {
	payload := htmlPayload("echo-value")
	text := string(payload)
	startHTML := parseHTMLOffset(t, text, "StartHTML")
	endHTML := parseHTMLOffset(t, text, "EndHTML")
	startFragment := parseHTMLOffset(t, text, "StartFragment")
	endFragment := parseHTMLOffset(t, text, "EndFragment")
	if startHTML != strings.Index(text, "<html>") {
		t.Fatalf("StartHTML = %d, want body offset", startHTML)
	}
	if endHTML != len(payload) || startFragment >= endFragment {
		t.Fatalf("unexpected HTML offsets: %d, %d, %d, %d", startHTML, endHTML, startFragment, endFragment)
	}
	if string(payload[startFragment:endFragment]) != "<b>echo-value</b>" {
		t.Fatalf("fragment = %q", payload[startFragment:endFragment])
	}
}

func TestDIBPayloadHasExpectedGeometry(t *testing.T) {
	payload := dibPayload()
	if len(payload) != 40+bitmapRowBytes*bitmapHeight {
		t.Fatalf("DIB length = %d", len(payload))
	}
	if got := binary.LittleEndian.Uint32(payload[0:4]); got != 40 {
		t.Fatalf("header size = %d", got)
	}
	if got := binary.LittleEndian.Uint32(payload[4:8]); got != bitmapWidth {
		t.Fatalf("width = %d", got)
	}
	if got := binary.LittleEndian.Uint32(payload[8:12]); got != bitmapHeight {
		t.Fatalf("height = %d", got)
	}
}

func TestDropFilesPayloadUsesWideDoubleTerminatedPath(t *testing.T) {
	path := `C:\Temp\echo-clipboard.txt`
	payload := dropFilesPayload(path)
	if got := binary.LittleEndian.Uint32(payload[0:4]); got != 20 {
		t.Fatalf("pFiles = %d", got)
	}
	if got := binary.LittleEndian.Uint32(payload[16:20]); got != 1 {
		t.Fatalf("fWide = %d", got)
	}
	words := make([]uint16, (len(payload)-20)/2)
	for index := range words {
		words[index] = binary.LittleEndian.Uint16(payload[20+index*2:])
	}
	if got := string(utf16.Decode(words)); got != path+"\x00\x00" {
		t.Fatalf("path payload = %q", got)
	}
}

func parseHTMLOffset(t *testing.T, text, name string) int {
	t.Helper()
	lineStart := strings.Index(text, name+":")
	if lineStart < 0 {
		t.Fatalf("HTML header does not contain %s", name)
	}
	lineEnd := strings.IndexByte(text[lineStart:], '\n')
	if lineEnd < 0 {
		t.Fatalf("HTML header line is incomplete for %s", name)
	}
	var value int
	if _, err := fmt.Sscanf(text[lineStart:lineStart+lineEnd], name+":%d", &value); err != nil {
		t.Fatalf("parse %s: %v", name, err)
	}
	return value
}
