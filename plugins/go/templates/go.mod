// Written by TinkerPad's Go plugin. Safe to edit; `go get` works too.
module {{package}}

go 1.21

require tinkerpad v0.0.0

// The dump library, from the Go plugin folder. The path is also recorded in
// playground.toml, in case the app moves and it needs repairing.
replace tinkerpad => {{plugin_dir}}/dump
