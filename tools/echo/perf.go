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
		"r5_storage_perf",
	)
}
