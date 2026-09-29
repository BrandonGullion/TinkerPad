package main

import "tinkerpad"

// {{name}}, created {{date}}. Mod+Enter runs it.

type person struct {
	Name string
	Born int
}

func main() {
	people := []person{
		{Name: "Ada", Born: 1815},
		{Name: "Grace", Born: 1906},
	}
	tinkerpad.Dump(people, "people")
}
