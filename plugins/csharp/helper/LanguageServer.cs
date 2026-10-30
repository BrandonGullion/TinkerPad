// The helper as a language server (work item 04-13): `--lsp` instead of the
// run protocol, over the same stdin and stdout.
//
// Roslyn's own language server was tried first and set aside: its completion
// list leaves out everything the implicit usings bring in — `Console.` comes
// back empty — and it has to be downloaded from a feed that isn't nuget.org
// (the findings are in the work item). This is the other way round: the
// compiler the helper already keeps warm, asked through Roslyn's editor
// services, with the playground compiled exactly as a run compiles it. So a
// squiggle and a run can't disagree about a playground.
//
// It is a small server on purpose. Full document sync, completion, hover,
// diagnostics pushed after each change, and go to definition — what the
// editor's client asks for, and nothing it doesn't (crates/tinkerpad-lsp,
// `client_capabilities`). Everything that touches the workspace happens on one
// queue, in order, so no request ever sees a half-applied edit.

using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Channels;

namespace TinkerPad.CSharp.Helper;

internal sealed class LanguageServer
{
    /// <summary>How long after the last keystroke diagnostics are worked out.</summary>
    private static readonly TimeSpan DiagnosticsDelay = TimeSpan.FromMilliseconds(250);

    private readonly Stream _input;
    private readonly Stream _output;
    private readonly object _writing = new();
    private readonly Channel<Func<Task>> _queue = Channel.CreateUnbounded<Func<Task>>(
        new UnboundedChannelOptions { SingleReader = true });
    private readonly Playgrounds _playgrounds;
    private bool _shuttingDown;

    private LanguageServer(Stream input, Stream output, string pluginDir, string scratchDir)
    {
        _input = input;
        _output = output;
        _playgrounds = new Playgrounds(pluginDir, scratchDir, Log, job => _queue.Writer.TryWrite(job));
    }

    /// <summary>`--lsp --plugin-dir DIR --scratch DIR`.</summary>
    public static int Run(string[] args)
    {
        string? Arg(string name)
        {
            var at = Array.IndexOf(args, name);
            return at >= 0 && at + 1 < args.Length ? args[at + 1] : null;
        }
        var pluginDir = Arg("--plugin-dir") ?? AppContext.BaseDirectory;
        var scratchDir = Arg("--scratch") ?? Path.Combine(Path.GetTempPath(), "tinkerpad-csharp-lsp");

        var server = new LanguageServer(
            Console.OpenStandardInput(),
            Console.OpenStandardOutput(),
            pluginDir,
            scratchDir);
        var worker = Task.Run(server.Work);
        var code = server.Read();
        server._queue.Writer.TryComplete();
        worker.Wait(TimeSpan.FromSeconds(2));
        return code;
    }

    // ---- the wire ------------------------------------------------------------

    /// <summary>Read messages until `exit` or the end of stdin.</summary>
    private int Read()
    {
        while (true)
        {
            var message = ReadMessage();
            if (message is null)
            {
                return _shuttingDown ? 0 : 1;
            }
            var method = message["method"]?.GetValue<string>();
            var id = message["id"]?.DeepClone();
            var parameters = message["params"];
            if (method == "exit")
            {
                return _shuttingDown ? 0 : 1;
            }
            if (method is null)
            {
                continue; // A reply to something we never ask.
            }
            _queue.Writer.TryWrite(() => Handle(method, id, parameters));
        }
    }

    /// <summary>One `Content-Length`-framed message, or null at the end.</summary>
    private JsonObject? ReadMessage()
    {
        var length = -1;
        while (true)
        {
            var line = ReadHeaderLine();
            if (line is null)
            {
                return null;
            }
            if (line.Length == 0)
            {
                break;
            }
            const string header = "Content-Length:";
            if (line.StartsWith(header, StringComparison.OrdinalIgnoreCase))
            {
                length = int.Parse(line[header.Length..].Trim());
            }
        }
        if (length < 0)
        {
            return null;
        }
        var body = new byte[length];
        var read = 0;
        while (read < length)
        {
            var n = _input.Read(body, read, length - read);
            if (n == 0)
            {
                return null;
            }
            read += n;
        }
        try
        {
            return JsonNode.Parse(body) as JsonObject;
        }
        catch (JsonException e)
        {
            Log($"unreadable message: {e.Message}");
            return new JsonObject();
        }
    }

    private string? ReadHeaderLine()
    {
        var bytes = new List<byte>();
        while (true)
        {
            var b = _input.ReadByte();
            if (b < 0)
            {
                return null;
            }
            if (b == '\n')
            {
                return Encoding.ASCII.GetString(bytes.ToArray()).TrimEnd('\r');
            }
            bytes.Add((byte)b);
        }
    }

    private void Send(JsonObject message)
    {
        message["jsonrpc"] = "2.0";
        var body = Encoding.UTF8.GetBytes(message.ToJsonString());
        var header = Encoding.ASCII.GetBytes($"Content-Length: {body.Length}\r\n\r\n");
        lock (_writing)
        {
            _output.Write(header);
            _output.Write(body);
            _output.Flush();
        }
    }

    private void Reply(JsonNode? id, JsonNode? result) =>
        Send(new JsonObject { ["id"] = id, ["result"] = result });

    private void Fail(JsonNode? id, int code, string message) =>
        Send(new JsonObject
        {
            ["id"] = id,
            ["error"] = new JsonObject { ["code"] = code, ["message"] = message },
        });

    private void Notify(string method, JsonNode parameters) =>
        Send(new JsonObject { ["method"] = method, ["params"] = parameters });

    /// <summary>Into the server's log in the status strip, and stderr.</summary>
    private void Log(string message)
    {
        Core.Warn(message);
        Notify("window/logMessage", new JsonObject { ["type"] = 3, ["message"] = message });
    }

    // ---- the work ------------------------------------------------------------

    private async Task Work()
    {
        await foreach (var job in _queue.Reader.ReadAllAsync())
        {
            try
            {
                await job();
            }
            catch (Exception e)
            {
                Log($"internal error: {e}");
            }
        }
    }

    private async Task Handle(string method, JsonNode? id, JsonNode? parameters)
    {
        try
        {
            switch (method)
            {
                case "initialize":
                    Reply(id, Initialize());
                    return;
                case "initialized":
                    // Roslyn's first completion in a process is the slow one:
                    // pay for it now, before anyone has typed anything.
                    await _playgrounds.Warm();
                    return;
                case "shutdown":
                    _shuttingDown = true;
                    Reply(id, null);
                    return;
                case "textDocument/didOpen":
                {
                    var document = parameters!["textDocument"]!;
                    Changed(
                        document["uri"]!.GetValue<string>(),
                        document["version"]?.GetValue<int>() ?? 0,
                        document["text"]!.GetValue<string>());
                    return;
                }
                case "textDocument/didChange":
                {
                    var document = parameters!["textDocument"]!;
                    // Full sync: the last change is the whole text.
                    var text = parameters["contentChanges"]!.AsArray().LastOrDefault()?["text"]?.GetValue<string>();
                    if (text is not null)
                    {
                        Changed(
                            document["uri"]!.GetValue<string>(),
                            document["version"]?.GetValue<int>() ?? 0,
                            text);
                    }
                    return;
                }
                case "textDocument/didClose":
                    _playgrounds.Close(parameters!["textDocument"]!["uri"]!.GetValue<string>());
                    return;
                case "textDocument/completion":
                    Reply(id, await _playgrounds.Complete(DocumentUri(parameters), Position(parameters)));
                    return;
                case "textDocument/hover":
                    Reply(id, await _playgrounds.Hover(DocumentUri(parameters), Position(parameters)));
                    return;
                case "textDocument/definition":
                    Reply(id, await _playgrounds.Definition(DocumentUri(parameters), Position(parameters)));
                    return;
                default:
                    if (id is not null)
                    {
                        Fail(id, -32601, $"{method} isn't something this server does");
                    }
                    return;
            }
        }
        catch (Exception e)
        {
            Log($"{method} failed: {e}");
            if (id is not null)
            {
                Fail(id, -32603, e.Message);
            }
        }
    }

    private static JsonObject Initialize() => new()
    {
        ["capabilities"] = new JsonObject
        {
            ["positionEncoding"] = "utf-16",
            ["textDocumentSync"] = new JsonObject { ["openClose"] = true, ["change"] = 1 },
            ["completionProvider"] = new JsonObject
            {
                ["triggerCharacters"] = new JsonArray(".", " ", "("),
                ["resolveProvider"] = false,
            },
            ["hoverProvider"] = true,
            ["definitionProvider"] = true,
        },
        ["serverInfo"] = new JsonObject { ["name"] = "tinkerpad-csharp-helper", ["version"] = "1.0.0" },
    };

    /// <summary>
    /// A document changed: update it now, and publish its diagnostics once the
    /// typing has paused — and only if nothing newer has arrived meanwhile.
    /// </summary>
    private void Changed(string uri, int version, string text)
    {
        _playgrounds.Update(uri, version, text, packagesReady: () =>
            // The packages a `#:package` line asked for arrived: the squiggles
            // that were waiting for them are worth redoing.
            _queue.Writer.TryWrite(() => Publish(uri)));
        _ = Task.Delay(DiagnosticsDelay).ContinueWith(_ =>
        {
            if (_playgrounds.VersionOf(uri) == version)
            {
                _queue.Writer.TryWrite(() => Publish(uri));
            }
        });
    }

    private async Task Publish(string uri)
    {
        var (version, diagnostics) = await _playgrounds.Diagnose(uri);
        if (diagnostics is null)
        {
            return; // Closed, or still waiting for its packages.
        }
        Notify("textDocument/publishDiagnostics", new JsonObject
        {
            ["uri"] = uri,
            ["version"] = version,
            ["diagnostics"] = diagnostics,
        });
    }

    private static string DocumentUri(JsonNode? parameters) =>
        parameters!["textDocument"]!["uri"]!.GetValue<string>();

    private static (int Line, int Character) Position(JsonNode? parameters)
    {
        var position = parameters!["position"]!;
        return (position["line"]!.GetValue<int>(), position["character"]!.GetValue<int>());
    }
}
