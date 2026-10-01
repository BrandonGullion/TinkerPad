// The table `tinkerpad plugin check` expects (docs/PLUGINS.md §8.1).
using TinkerPad;

new[] { new { Name = "ada", N = 1 }, new { Name = "grace", N = 2 } }.Dump("check");
