package main

// The table `tinkerpad plugin check` expects (docs/PLUGINS.md §8.1).

import "tinkerpad"

type row struct {
	Name string
	N    int
}

func main() {
	rows := []row{{Name: "ada", N: 1}, {Name: "grace", N: 2}}
	tinkerpad.Dump(rows, "check")
}
