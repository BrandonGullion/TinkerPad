// Package tinkerpad shows a value in TinkerPad's output pane.
//
// One function. It writes the sentinel described in docs/PLUGINS.md §6 — an
// OSC sequence carrying JSON — to standard output, where the app reads it out
// of the stream and renders it. Printed anywhere else (a terminal, a pipe) the
// sequence is invisible, so a playground still behaves like a Go program.
package tinkerpad

import (
	"encoding/json"
	"fmt"
	"os"
	"reflect"
)

// Dump shows a value. An optional title names it in the pane:
//
//	tinkerpad.Dump(people)
//	tinkerpad.Dump(people, "people")
//
// A slice of structs or maps becomes a table, one row each; anything else
// becomes a tree, which the pane shows collapsible with the first two levels
// open. Values that can't be turned into JSON (a channel, a func) are shown as
// their Go representation instead of failing the run.
func Dump(value any, title ...string) {
	node := describe(value)
	if len(title) > 0 && title[0] != "" {
		node["title"] = title[0]
	}
	encoded, err := json.Marshal(node)
	if err != nil {
		encoded, _ = json.Marshal(map[string]any{
			"v":     1,
			"kind":  "text",
			"text":  fmt.Sprintf("%#v", value),
			"title": fmt.Sprintf("%T (not JSON: %v)", value, err),
		})
	}
	// ESC ] 1337 ; TinkerPad= <json> BEL
	fmt.Fprintf(os.Stdout, "\x1b]1337;TinkerPad=%s\x07", encoded)
}

// describe picks the shape that suits the value.
func describe(value any) map[string]any {
	node := map[string]any{"v": 1}
	if value == nil {
		node["kind"] = "value"
		node["value"] = nil
		node["type"] = "nil"
		return node
	}
	typeName := fmt.Sprintf("%T", value)

	switch reflect.ValueOf(value).Kind() {
	case reflect.Bool, reflect.String,
		reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64,
		reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64,
		reflect.Float32, reflect.Float64:
		node["kind"] = "value"
		node["value"] = value
		node["type"] = typeName
		return node
	case reflect.Slice, reflect.Array:
		if table := asTable(reflect.ValueOf(value)); table != nil {
			return table
		}
	}

	node["kind"] = "tree"
	node["value"] = value
	node["type"] = typeName
	return node
}

// asTable turns a slice of structs or string-keyed maps into a table, or
// returns nil when the rows have no columns to speak of.
func asTable(rows reflect.Value) map[string]any {
	if rows.Len() == 0 {
		return nil
	}
	names := columns(rows.Index(0))
	if len(names) == 0 {
		return nil
	}

	columnNodes := make([]map[string]any, 0, len(names))
	for _, name := range names {
		columnNodes = append(columnNodes, map[string]any{"name": name, "type": ""})
	}

	cells := make([][]any, 0, rows.Len())
	for i := 0; i < rows.Len(); i++ {
		row := indirect(rows.Index(i))
		// A row shaped differently from the first isn't a table row.
		if !sameShape(row, names) {
			return nil
		}
		cells = append(cells, values(row, names))
	}

	return map[string]any{
		"v":         1,
		"kind":      "table",
		"columns":   columnNodes,
		"rows":      cells,
		"truncated": false,
	}
}

// columns names the fields of a struct, or the keys of a string-keyed map.
func columns(row reflect.Value) []string {
	row = indirect(row)
	switch row.Kind() {
	case reflect.Struct:
		var names []string
		for i := 0; i < row.NumField(); i++ {
			field := row.Type().Field(i)
			if field.IsExported() {
				names = append(names, field.Name)
			}
		}
		return names
	case reflect.Map:
		if row.Type().Key().Kind() != reflect.String {
			return nil
		}
		var names []string
		for _, key := range row.MapKeys() {
			names = append(names, key.String())
		}
		// Map order is random in Go; sorted is at least the same every run.
		sortStrings(names)
		return names
	default:
		return nil
	}
}

func sameShape(row reflect.Value, names []string) bool {
	switch row.Kind() {
	case reflect.Struct, reflect.Map:
		return len(columns(row)) == len(names)
	default:
		return false
	}
}

func values(row reflect.Value, names []string) []any {
	cells := make([]any, 0, len(names))
	for _, name := range names {
		switch row.Kind() {
		case reflect.Struct:
			cells = append(cells, row.FieldByName(name).Interface())
		case reflect.Map:
			value := row.MapIndex(reflect.ValueOf(name))
			if value.IsValid() {
				cells = append(cells, value.Interface())
			} else {
				cells = append(cells, nil)
			}
		}
	}
	return cells
}

func indirect(value reflect.Value) reflect.Value {
	for value.Kind() == reflect.Ptr || value.Kind() == reflect.Interface {
		if value.IsNil() {
			return value
		}
		value = value.Elem()
	}
	return value
}

// sortStrings is a three-line insertion sort, so this package keeps its one
// virtue: it imports nothing a playground doesn't already have.
func sortStrings(items []string) {
	for i := 1; i < len(items); i++ {
		for j := i; j > 0 && items[j] < items[j-1]; j-- {
			items[j], items[j-1] = items[j-1], items[j]
		}
	}
}
