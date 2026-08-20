package main

func (a *app) perf() error {
	return a.run(
		"cargo",
		"run",
		"--quiet",
		"--locked",
		"--manifest-path",
		"crates/echo-storage/Cargo.toml",
		"--example",
		"storage_perf",
	)
}
