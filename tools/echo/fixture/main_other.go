//go:build !windows

package main

import (
	"fmt"
	"os"
)

func main() {
	_, _ = fmt.Fprintln(os.Stderr, "echo-native-fixture is only supported on Windows")
	os.Exit(1)
}
