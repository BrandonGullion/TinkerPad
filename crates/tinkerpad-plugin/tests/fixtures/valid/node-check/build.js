// The test plugin's `[helper.build]`: a stand-in for `dotnet build`.
//
// It is given the helper.s scratch folder as its one argument, and writes a
// line into the helper's scratch folder saying it ran, so the
// self-tests can see that the build happened before the first use and didn't
// happen again afterwards (work item 03-02). A real build would compile the
// helper; this one only has to take a moment and say something.

"use strict";

const fs = require("fs");
const path = require("path");

const scratch = process.argv[2];
console.log("building the node-check helper");
if (scratch) {
  fs.mkdirSync(scratch, { recursive: true });
  fs.appendFileSync(path.join(scratch, "builds.txt"), `${new Date().toISOString()}\n`);
  console.log(`marked ${path.join(scratch, "builds.txt")}`);
}
