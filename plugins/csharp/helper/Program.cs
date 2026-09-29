// TinkerPad's C# helper (work item 03-05): Roslyn, kept warm.
//
// The per-run path for C# is `dotnet run --file main.cs`, which is a second or
// two: MSBuild, a restore check, a compiler process, a new runtime. This helper
// is the same work done once. It stays up, holds the compiler and the .NET
// reference set in memory, and each run is a parse, a compile, and an assembly
// load into a collectible context — tens of milliseconds.
//
// It speaks `tinkerpad-plugin-v1` on stdin and stdout (docs/PLUGINS.md §5).
// `Protocol.cs` is the wire, `Runner.cs` does the work, and this file is the
// loop that joins them. The Python helper next door is the short version of
// the same thing, and is the one to read first.

using System.Text;
using System.Text.Json.Nodes;

namespace TinkerPad.CSharp.Helper;

internal static class Program
{
    private static int Main()
    {
        // The protocol is UTF-8, and so is everything a playground prints.
        var output = new StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false))
        {
            AutoFlush = false,
        };
        var input = new StreamReader(Console.OpenStandardInput(), new UTF8Encoding(false));
        Console.OutputEncoding = new UTF8Encoding(false);

        var core = new Core(input, output);
        Runner? runner = null;

        while (true)
        {
            var message = core.Read();
            if (message is null)
            {
                return 0; // The core closed our stdin: it is done with us.
            }

            switch (message.Type)
            {
                case "hello":
                    runner = new Runner(core, message.Text("plugin_dir") ?? "");
                    Warm(runner, message.Text("scratch_dir"));
                    core.Ready("tinkerpad-csharp-helper", "1.0.0");
                    break;

                case "run" when runner is not null:
                    var main = message.Text("main");
                    var directory = message.Text("dir");
                    if (main is null || directory is null)
                    {
                        core.Failed(message.Id, "The run said nothing about what to run.", []);
                        break;
                    }
                    runner.Start(message.Id, main, directory);
                    break;

                case "stdin" when runner is not null:
                    runner.Typed(message.Id, message.Data() ?? "");
                    break;

                case "stdin_eof" when runner is not null:
                    runner.InputEnded(message.Id);
                    break;

                case "cancel" when runner is not null:
                    runner.Cancel(message.Id);
                    break;

                case "shutdown":
                    return 0;

                default:
                    // A message this version doesn't know: ignore it and carry
                    // on, so a newer TinkerPad can talk to an older helper.
                    Core.Warn($"ignoring {message.Type}");
                    break;
            }
        }
    }

    /// <summary>
    /// Compile something trivial before saying `ready`, so the first real run
    /// doesn't pay for Roslyn's own start-up.
    /// </summary>
    /// <remarks>
    /// This is the difference between a first run of ~1 s and one of ~100 ms.
    /// It happens while the core is still waiting for `ready`, which it allows
    /// ten seconds for.
    /// </remarks>
    private static void Warm(Runner runner, string? scratchDirectory)
    {
        try
        {
            var directory = string.IsNullOrEmpty(scratchDirectory)
                ? Path.Combine(Path.GetTempPath(), "tinkerpad-csharp-helper")
                : scratchDirectory;
            Directory.CreateDirectory(directory);
            var warmup = Path.Combine(directory, "warmup.cs");
            File.WriteAllText(warmup, "System.Diagnostics.Debug.WriteLine(\"warm\");\n");
            runner.Prepare(warmup);
        }
        catch (Exception e)
        {
            // Not being warm is slow, not broken.
            Core.Warn($"could not warm up: {e.Message}");
        }
    }
}
