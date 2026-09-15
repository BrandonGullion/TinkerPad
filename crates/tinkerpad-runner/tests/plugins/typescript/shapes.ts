import { dump } from "tinkerpad";

// Every shape the TypeScript plugin's dump library writes. The output is
// captured in tests/dump_cases/13-typescript-shapes.txt, so keep it the same on
// every machine: no times, paths, or stack traces.

enum Level {
  Low = 1,
  High = 2,
}

class Point {
  constructor(
    public x: number,
    public y: number,
  ) {}
}

console.log("table:");
dump(
  [
    { id: 1, name: "Ada", tags: ["math"] },
    { id: 2, name: "Grace", born: 1906 },
  ],
  "people",
);

console.log("table with a nested table:");
dump([{ team: "core", members: [{ name: "Ada" }, { name: "Alan" }] }]);

console.log("tree:");
dump({ name: "TinkerPad", level: Level.High, point: new Point(1, 2), list: [1, "two", null] }, "config");

console.log("values:");
dump(42);
dump("text");
dump(true);
dump(null);
dump(undefined);
dump(10n);
dump(Number.NaN);
dump(new Date(Date.UTC(2026, 8, 15)));
dump(/re/g);
dump(function named() {});
dump(Symbol("s"));

console.log("map and set:");
dump(
  new Map([
    ["a", 1],
    ["b", 2],
  ]),
);
dump(new Map([[{ id: 1 }, "object key"]]));
dump(new Set(["x", "y"]));

console.log("cycle and depth:");
const loop: { name: string; self?: unknown } = { name: "loop" };
loop.self = loop;
dump(loop);
dump({ l1: { l2: { l3: { l4: { l5: { l6: true } } } } } });

console.log("chained:", dump([1, 2, 3]).length);
