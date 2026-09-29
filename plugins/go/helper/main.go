// The Go plugin's helper: one job, and it isn't running code.
//
// gopls works across a set of modules only if a `go.work` names them, and it
// reads that file when it starts. TinkerPad tells a plugin when a playground
// is opened or closed (docs/PLUGINS.md §5), so this answers those two hooks by
// writing the workspace file listing every Go playground currently open, plus
// the plugin's own dump module so `replace` targets resolve.
//
// Runs are not its business: `go build` caches well enough that a warm runner
// would buy nothing (work item 04-07).
package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

type message struct {
	Type       string       `json:"type"`
	ID         string       `json:"id"`
	ScratchDir string       `json:"scratch_dir"`
	PluginDir  string       `json:"plugin_dir"`
	Playground *playground  `json:"playground"`
	Open       []playground `json:"open"`
}

type playground struct {
	ID   string `json:"id"`
	Name string `json:"name"`
	Dir  string `json:"dir"`
}

// Where go.work goes and what it must always include, learned from `hello`.
var (
	scratchDir string
	pluginDir  string
)

func main() {
	in := bufio.NewScanner(os.Stdin)
	// A playground path can be long; the default 64 KiB line limit is plenty,
	// but say so rather than truncating silently.
	in.Buffer(make([]byte, 0, 64*1024), 1024*1024)

	for in.Scan() {
		line := strings.TrimSpace(in.Text())
		if line == "" {
			continue
		}
		var m message
		if err := json.Unmarshal([]byte(line), &m); err != nil {
			warn("not a message: %v", err)
			continue
		}

		switch m.Type {
		case "hello":
			scratchDir, pluginDir = m.ScratchDir, m.PluginDir
			send(map[string]any{
				"type":    "ready",
				"name":    "tinkerpad-go-helper",
				"version": "1.0.0",
			})
		case "playground_opened", "playground_closed":
			// The core sends every open playground of this language, this one
			// included for `opened` and excluded for `closed`, so the list is
			// the answer and nothing has to be remembered between messages.
			if err := writeWorkspace(m.Open); err != nil {
				warn("could not write go.work: %v", err)
			}
			send(map[string]any{"type": "result", "id": m.ID})
		case "shutdown":
			return
		default:
			// A message this helper doesn't know: ignore it and carry on, so a
			// newer TinkerPad can talk to an older helper.
			warn("ignoring %s", m.Type)
		}
	}
}

// writeWorkspace rewrites `<scratch>/go.work` to use every open playground.
//
// Written whole and replaced atomically: gopls watches this file, and half a
// workspace is worse than a stale one.
func writeWorkspace(open []playground) error {
	if scratchDir == "" {
		return fmt.Errorf("no scratch folder; `hello` never arrived")
	}
	if err := os.MkdirAll(scratchDir, 0o755); err != nil {
		return err
	}

	dirs := make([]string, 0, len(open)+1)
	if pluginDir != "" {
		// The dump module every playground's `replace` points at.
		dirs = append(dirs, filepath.Join(pluginDir, "dump"))
	}
	for _, p := range open {
		if p.Dir != "" {
			dirs = append(dirs, p.Dir)
		}
	}
	sort.Strings(dirs)

	var text strings.Builder
	text.WriteString("// Written by TinkerPad's Go plugin as playgrounds open and close.\n")
	text.WriteString("// Editing it is pointless: it is rewritten on the next tab change.\n\n")
	text.WriteString("go 1.21\n\n")
	switch len(dirs) {
	case 0:
		// `use ()` is valid and means what it says: a workspace of nothing.
		text.WriteString("use ()\n")
	default:
		text.WriteString("use (\n")
		for _, dir := range dirs {
			fmt.Fprintf(&text, "\t%s\n", filepath.ToSlash(dir))
		}
		text.WriteString(")\n")
	}

	final := filepath.Join(scratchDir, "go.work")
	temp := final + ".tmp"
	if err := os.WriteFile(temp, []byte(text.String()), 0o644); err != nil {
		return err
	}
	return os.Rename(temp, final)
}

func send(m map[string]any) {
	encoded, err := json.Marshal(m)
	if err != nil {
		warn("could not encode %v: %v", m["type"], err)
		return
	}
	fmt.Fprintf(os.Stdout, "%s\n", encoded)
}

func warn(format string, args ...any) {
	fmt.Fprintf(os.Stderr, "go helper: "+format+"\n", args...)
}
