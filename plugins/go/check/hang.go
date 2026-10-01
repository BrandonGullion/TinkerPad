package main

// Runs until it is cancelled.

import (
	"fmt"
	"time"
)

func main() {
	fmt.Println("started")
	for {
		time.Sleep(50 * time.Millisecond)
	}
}
