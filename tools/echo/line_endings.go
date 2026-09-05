package main

import "bytes"

func canonicalizeLineEndings(input []byte) []byte {
	canonical := bytes.ReplaceAll(input, []byte{'\r', '\n'}, []byte{'\n'})
	return bytes.ReplaceAll(canonical, []byte{'\r'}, []byte{'\n'})
}
