// The wire format of `tinkerpad-plugin-v1` (docs/PLUGINS.md §5): one JSON
// object per line, in and out.
//
// Nothing here knows what a run is. Reading and writing messages is separate
// from doing anything with them, so `Program` reads like the protocol it
// implements.

using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace TinkerPad.CSharp.Helper;

/// <summary>One message from the core. Anything not asked about is ignored.</summary>
internal sealed record Message(string Type, JsonObject Body)
{
    public string Id => Body["id"]?.GetValue<string>() ?? "";

    public string? Text(string name) => Body[name]?.GetValue<string>();

    /// <summary>
    /// A `data` field, which the protocol carries as base64 (work item 04-05).
    /// Nonsense decodes to nothing rather than throwing: a helper that dies on
    /// a bad message is worse than one that ignores it.
    /// </summary>
    public string? Data()
    {
        var raw = Text("data");
        if (raw is null)
        {
            return null;
        }
        try
        {
            return Encoding.UTF8.GetString(Convert.FromBase64String(raw));
        }
        catch (FormatException)
        {
            Core.Warn("a stdin message was not base64");
            return null;
        }
    }
}

/// <summary>The core, on the other end of stdin and stdout.</summary>
internal sealed class Core
{
    private readonly TextReader _in;
    private readonly TextWriter _out;
    private readonly object _writing = new();

    public Core(TextReader input, TextWriter output)
    {
        _in = input;
        _out = output;
    }

    /// <summary>The next message, or null when the core has closed stdin.</summary>
    public Message? Read()
    {
        while (true)
        {
            var line = _in.ReadLine();
            if (line is null)
            {
                return null;
            }
            if (line.Trim().Length == 0)
            {
                continue;
            }
            JsonObject? body;
            try
            {
                body = JsonNode.Parse(line) as JsonObject;
            }
            catch (JsonException)
            {
                Warn($"not a message: {Shorten(line)}");
                continue;
            }
            var type = body?["type"]?.GetValue<string>();
            if (body is null || type is null)
            {
                Warn($"not a message: {Shorten(line)}");
                continue;
            }
            return new Message(type, body);
        }
    }

    /// <summary>
    /// Send one message. Locked because a run's output arrives from the thread
    /// the playground is on while the protocol thread may be replying.
    /// </summary>
    public void Send(JsonObject message)
    {
        lock (_writing)
        {
            _out.Write(message.ToJsonString());
            _out.Write('\n');
            _out.Flush();
        }
    }

    public void Ready(string name, string version) =>
        Send(new JsonObject { ["type"] = "ready", ["name"] = name, ["version"] = version });

    /// <summary>A chunk of a run's output, as the protocol wants it: base64.</summary>
    public void Output(string kind, string runId, byte[] bytes)
    {
        if (bytes.Length == 0)
        {
            return;
        }
        Send(new JsonObject
        {
            ["type"] = kind,
            ["id"] = runId,
            ["data"] = Convert.ToBase64String(bytes),
        });
    }

    public void Output(string kind, string runId, string text) =>
        Output(kind, runId, Encoding.UTF8.GetBytes(text));

    public void Exited(string runId, int? code, long ms) =>
        Send(new JsonObject
        {
            ["type"] = "exited",
            ["id"] = runId,
            ["code"] = code,
            ["duration_ms"] = ms,
        });

    /// <summary>The program never ran: a compile error, with where to point.</summary>
    public void Failed(string runId, string message, IEnumerable<JsonObject> diagnostics)
    {
        var array = new JsonArray();
        foreach (var diagnostic in diagnostics)
        {
            array.Add(diagnostic);
        }
        Send(new JsonObject
        {
            ["type"] = "failed",
            ["id"] = runId,
            ["message"] = message,
            ["diagnostics"] = array,
        });
    }

    /// <summary>The helper's own stderr: for the log, never for the results pane.</summary>
    public static void Warn(string message) => Console.Error.WriteLine($"helper: {message}");

    private static string Shorten(string line) => line.Length <= 200 ? line : line[..200];
}
