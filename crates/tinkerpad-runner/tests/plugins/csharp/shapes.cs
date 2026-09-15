// Every shape the C# plugin's dump library writes. The output is captured in
// tests/dump_cases/14-csharp-shapes.txt, so keep it the same on every machine:
// no times, paths, or stack traces, and "\n" rather than Console.WriteLine
// (which writes "\r\n" on Windows).
using TinkerPad;

Console.Write("table:\n");
new[] { new Person("Ada", 1815, ["math"]), new Person("Grace", 1906, null) }.Dump("people");

Console.Write("table with a nested table:\n");
new[] { new { Team = "core", Members = new[] { new { Name = "Ada" }, new { Name = "Alan" } } } }.Dump();

Console.Write("tree:\n");
new { Name = "TinkerPad", Level = Level.High, Point = (X: 1, Y: 2), List = new object?[] { 1, "two", null } }.Dump("config");

Console.Write("values:\n");
42.Dump();
"text".Dump();
true.Dump();
((string?)null).Dump();
3.5m.Dump();
double.NaN.Dump();
ulong.MaxValue.Dump();
'c'.Dump();
Level.Low.Dump();
new DateTime(2026, 9, 15, 0, 0, 0, DateTimeKind.Utc).Dump();
new DateOnly(2026, 9, 15).Dump();
TimeSpan.FromMinutes(90).Dump();
Guid.Empty.Dump();
typeof(List<int>).Dump();

Console.Write("dictionaries and sets:\n");
new Dictionary<string, int> { ["a"] = 1, ["b"] = 2 }.Dump();
new Dictionary<Person, string> { [new Person("Key", 1, null)] = "object key" }.Dump();
new HashSet<string> { "x", "y" }.Dump();

Console.Write("cycle, depth, and errors:\n");
var loop = new Node("loop");
loop.Next = loop;
loop.Dump();
new { L1 = new { L2 = new { L3 = new { L4 = new { L5 = new { L6 = true } } } } } }.Dump();
new Throws().Dump();

Console.Write("chained: " + new[] { 1, 2, 3 }.Dump().Length + "\n");

record Person(string Name, int Born, string[]? Tags);

class Node(string name)
{
    public string Name { get; } = name;
    public Node? Next { get; set; }
}

enum Level { Low, High }

class Throws
{
    public int Ok => 1;
    public int Bad => throw new InvalidOperationException("nope");
}
