// The table `tinkerpad plugin check` expects (docs/PLUGINS.md §8.1).
import { dump } from "tinkerpad";

dump([{ name: "ada", n: 1 }, { name: "grace", n: 2 }], "check");
