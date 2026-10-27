// The playgrounds the language server has open, as Roslyn projects (work item
// 04-13).
//
// Each open playground is a project of its own in one AdhocWorkspace: its
// main file, the implicit usings and the dump library — the same three things
// a run compiles, with the same options (Runner.cs) — against the same
// framework assemblies, plus whatever its `#:package` lines ask for. The
// framework's metadata is read once and shared by every project, so a second
// playground costs its own source and little else.
//
// The `#:` lines are blanked rather than removed, as a run does (Sources.cs),
// so line N here is line N in the editor and no position is ever translated.
//
// Everything here runs on the server's one queue, except fetching packages,
// which can take seconds and needs the network: that goes to the thread pool,
// and the result is put back on the queue.

using System.Collections.Concurrent;
using System.Collections.Immutable;
using System.Text;
using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.Completion;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.FindSymbols;
using Microsoft.CodeAnalysis.Host.Mef;
using Microsoft.CodeAnalysis.QuickInfo;
using Microsoft.CodeAnalysis.Tags;
using Microsoft.CodeAnalysis.Text;

namespace TinkerPad.CSharp.Helper;

internal sealed class Playgrounds
{
    /// <summary>How many completions go back at most. More is noise and bytes.</summary>
    private const int MaxCompletions = 400;

    private sealed class Open
    {
        public required string Uri;
        public required string Path;
        public required ProjectId Project;
        public required DocumentId Main;
        public int Version;
        /// <summary>The `#:package` lines the references are for, as text.</summary>
        public string Packages = "";
        /// <summary>A fetch is under way; diagnostics wait for it.</summary>
        public bool Fetching;
        /// <summary>Why the packages couldn't be had, if they couldn't.</summary>
        public string? PackageProblem;
    }

    private readonly AdhocWorkspace _workspace = new(MefHostServices.Create(MefHostServices.DefaultAssemblies));
    /// <summary>With XML docs, for hover. Not thread-safe: always under <see cref="_referencing"/>.</summary>
    private readonly References _references = new(documented: true);
    private readonly object _referencing = new();
    private readonly ConcurrentDictionary<string, Open> _open = new(StringComparer.Ordinal);
    private readonly SourceText _usings = SourceText.From(Runner.ImplicitUsings, Encoding.UTF8);
    private readonly string _dumpPath;
    private readonly SourceText? _dump;
    private readonly string _metadataDir;
    private readonly Action<string> _log;
    private readonly Action<Func<Task>> _post;

    /// <param name="post">Put work on the server's queue.</param>
    public Playgrounds(string pluginDir, string scratchDir, Action<string> log, Action<Func<Task>> post)
    {
        _log = log;
        _post = post;
        _dumpPath = Path.Combine(pluginDir, "dump", "TinkerPad.Dump.cs");
        _dump = File.Exists(_dumpPath) ? SourceText.From(File.ReadAllText(_dumpPath), Encoding.UTF8) : null;
        if (_dump is null)
        {
            log($"no dump library at {_dumpPath}; Dump() will be unknown");
        }
        _metadataDir = Path.Combine(scratchDir, "metadata");
    }

    // ---- documents -----------------------------------------------------------

    public int? VersionOf(string uri) => _open.TryGetValue(uri, out var open) ? open.Version : null;

    /// <summary>
    /// Open or update a document. `packagesReady` is called, on the queue, when
    /// packages it asked for have arrived.
    /// </summary>
    public void Update(string uri, int version, string text, Action packagesReady)
    {
        var source = Sources.Blank(SourceText.From(text, Encoding.UTF8), out var directives);
        var packages = string.Join(";", directives.Select(d => $"{d.Id}@{d.Version}"));

        if (!_open.TryGetValue(uri, out var open))
        {
            var path = new Uri(uri).LocalPath;
            open = Create(uri, path, source);
            _open[uri] = open;
        }
        else
        {
            var solution = _workspace.CurrentSolution.WithDocumentText(open.Main, source);
            _workspace.TryApplyChanges(solution);
        }
        open.Version = version;

        if (packages != open.Packages)
        {
            open.Packages = packages;
            Fetch(open, directives, packagesReady);
        }
    }

    public void Close(string uri)
    {
        if (_open.TryRemove(uri, out var open))
        {
            _workspace.TryApplyChanges(_workspace.CurrentSolution.RemoveProject(open.Project));
        }
    }

    private Open Create(string uri, string path, SourceText source)
    {
        var project = ProjectId.CreateNewId();
        var main = DocumentId.CreateNewId(project);
        var documents = new List<DocumentInfo>
        {
            Document(DocumentId.CreateNewId(project), "GlobalUsings.g.cs", _usings, null),
            Document(main, Path.GetFileName(path), source, path),
        };
        if (_dump is not null)
        {
            documents.Add(Document(DocumentId.CreateNewId(project), "TinkerPad.Dump.cs", _dump, _dumpPath));
        }
        IReadOnlyList<MetadataReference> framework;
        lock (_referencing)
        {
            framework = _references.Framework;
        }
        var info = ProjectInfo.Create(
            project,
            VersionStamp.Create(),
            name: "main",
            assemblyName: "main",
            LanguageNames.CSharp,
            filePath: path + ".csproj",
            compilationOptions: Runner.Options,
            parseOptions: Runner.Parsing,
            documents: documents,
            metadataReferences: framework);
        _workspace.AddProject(info);
        return new Open { Uri = uri, Path = path, Project = project, Main = main };
    }

    private static DocumentInfo Document(DocumentId id, string name, SourceText text, string? path) =>
        DocumentInfo.Create(
            id,
            name,
            loader: TextLoader.From(TextAndVersion.Create(text, VersionStamp.Create())),
            filePath: path);

    /// <summary>
    /// Get the packages the `#:package` lines name, off the queue: the first
    /// time it is a download. Afterwards they come from the global packages
    /// folder, so this works offline.
    /// </summary>
    private void Fetch(Open open, IReadOnlyList<PackageDirective> directives, Action packagesReady)
    {
        var wanted = open.Packages;
        open.Fetching = directives.Count > 0;
        if (directives.Count == 0)
        {
            Apply(open, wanted, null, null);
            return;
        }
        _log($"Restoring {string.Join(", ", directives.Select(d => d.Version is null ? d.Id : $"{d.Id} {d.Version}"))}…");
        _ = Task.Run(() =>
        {
            IReadOnlyList<MetadataReference> references;
            string? problem;
            lock (_referencing)
            {
                references = _references.For(directives, CancellationToken.None, out _, out problem);
            }
            _post(() =>
            {
                if (_open.TryGetValue(open.Uri, out var still) && still.Packages == wanted)
                {
                    Apply(still, wanted, references, problem);
                    _log(problem is null ? "Packages restored." : $"Packages: {problem}");
                    packagesReady();
                }
                return Task.CompletedTask;
            });
        });
    }

    private void Apply(Open open, string packages, IReadOnlyList<MetadataReference>? references, string? problem)
    {
        if (references is null)
        {
            lock (_referencing)
            {
                references = _references.Framework;
            }
        }
        var solution = _workspace.CurrentSolution.WithProjectMetadataReferences(open.Project, references);
        _workspace.TryApplyChanges(solution);
        open.Packages = packages;
        open.Fetching = false;
        open.PackageProblem = problem;
    }

    private Document? DocumentFor(string uri) =>
        _open.TryGetValue(uri, out var open) ? _workspace.CurrentSolution.GetDocument(open.Main) : null;

    private static async Task<int?> Offset(Document document, (int Line, int Character) position)
    {
        var text = await document.GetTextAsync();
        if (position.Line < 0 || position.Line >= text.Lines.Count)
        {
            return null;
        }
        var line = text.Lines[position.Line];
        return line.Start + Math.Clamp(position.Character, 0, line.End - line.Start);
    }

    // ---- diagnostics ---------------------------------------------------------

    /// <summary>
    /// The errors and warnings a run would report, at the same lines and
    /// columns. Null while packages are still arriving: a squiggle under every
    /// use of a package that is on its way would be worse than none.
    /// </summary>
    public async Task<(int Version, JsonArray? Diagnostics)> Diagnose(string uri)
    {
        if (!_open.TryGetValue(uri, out var open) || open.Fetching)
        {
            return (0, null);
        }
        var document = _workspace.CurrentSolution.GetDocument(open.Main);
        var model = document is null ? null : await document.GetSemanticModelAsync();
        if (model is null)
        {
            return (0, null);
        }
        var found = new JsonArray();
        if (open.PackageProblem is not null)
        {
            found.Add(Diagnostic(new LinePositionSpan(new(0, 0), new(0, 0)), 1, "TP0001", open.PackageProblem));
        }
        foreach (var diagnostic in model.GetDiagnostics())
        {
            if (diagnostic.IsSuppressed
                || diagnostic.Severity is not (DiagnosticSeverity.Error or DiagnosticSeverity.Warning))
            {
                continue;
            }
            found.Add(Diagnostic(
                diagnostic.Location.GetLineSpan().Span,
                diagnostic.Severity == DiagnosticSeverity.Error ? 1 : 2,
                diagnostic.Id,
                diagnostic.GetMessage()));
        }
        return (open.Version, found);
    }

    private static JsonObject Diagnostic(LinePositionSpan span, int severity, string code, string message) => new()
    {
        ["range"] = Range(span),
        ["severity"] = severity,
        ["code"] = code,
        ["source"] = "csharp",
        ["message"] = message,
    };

    // ---- completion ----------------------------------------------------------

    public async Task<JsonNode?> Complete(string uri, (int Line, int Character) position)
    {
        var document = DocumentFor(uri);
        var offset = document is null ? null : await Offset(document, position);
        if (document is null || offset is null)
        {
            return null;
        }
        var service = CompletionService.GetService(document);
        if (service is null)
        {
            return null;
        }
        var list = await service.GetCompletionsAsync(document, offset.Value);
        var text = await document.GetTextAsync();

        // What has been typed of the word so far. Roslyn's list is everything
        // that fits here; the editor filters as you type, but a thousand
        // items over the wire per keystroke is a thousand too many.
        var typed = text.ToString(TextSpan.FromBounds(list.Span.Start, Math.Max(list.Span.Start, offset.Value)));
        var items = list.ItemsList
            .Where(item => typed.Length == 0 || Matches(item.FilterText, typed))
            .Take(MaxCompletions + 1)
            .ToList();
        var incomplete = items.Count > MaxCompletions;

        var result = new JsonArray();
        foreach (var item in items.Take(MaxCompletions))
        {
            var insert = item.Properties.TryGetValue("InsertionText", out var insertion) ? insertion : item.DisplayText;
            var span = item.Span;
            var end = Math.Max(span.End, offset.Value);
            var entry = new JsonObject
            {
                ["label"] = item.DisplayTextPrefix + item.DisplayText + item.DisplayTextSuffix,
                ["kind"] = Kind(item.Tags),
                ["sortText"] = item.SortText,
                ["filterText"] = item.FilterText,
                ["textEdit"] = new JsonObject
                {
                    ["range"] = Range(text.Lines.GetLinePositionSpan(TextSpan.FromBounds(span.Start, end))),
                    ["newText"] = insert,
                },
            };
            if (!string.IsNullOrEmpty(item.InlineDescription))
            {
                entry["detail"] = item.InlineDescription;
            }
            result.Add(entry);
        }
        // Incomplete while the word is short or the list was cut, so the
        // editor asks again as it grows.
        return new JsonObject { ["isIncomplete"] = incomplete || typed.Length == 0, ["items"] = result };
    }

    /// <summary>Starts with what was typed, or has it in its camel humps.</summary>
    private static bool Matches(string candidate, string typed)
    {
        if (candidate.StartsWith(typed, StringComparison.OrdinalIgnoreCase))
        {
            return true;
        }
        var at = 0;
        foreach (var c in candidate)
        {
            if (at < typed.Length && char.ToLowerInvariant(c) == char.ToLowerInvariant(typed[at]))
            {
                at++;
            }
        }
        return at == typed.Length && char.ToLowerInvariant(candidate[0]) == char.ToLowerInvariant(typed[0]);
    }

    /// <summary>Roslyn's glyph tags, as LSP's completion kinds.</summary>
    private static int Kind(ImmutableArray<string> tags)
    {
        foreach (var tag in tags)
        {
            switch (tag)
            {
                case WellKnownTags.Method: return 2;
                case WellKnownTags.ExtensionMethod: return 2;
                case WellKnownTags.Property: return 10;
                case WellKnownTags.Field: return 5;
                case WellKnownTags.Local: return 6;
                case WellKnownTags.Parameter: return 6;
                case WellKnownTags.RangeVariable: return 6;
                case WellKnownTags.Class: return 7;
                case WellKnownTags.Interface: return 8;
                case WellKnownTags.Namespace: return 9;
                case WellKnownTags.Enum: return 13;
                case WellKnownTags.EnumMember: return 20;
                case WellKnownTags.Keyword: return 14;
                case WellKnownTags.Snippet: return 15;
                case WellKnownTags.Constant: return 21;
                case WellKnownTags.Structure: return 22;
                case WellKnownTags.Event: return 23;
                case WellKnownTags.Operator: return 24;
                case WellKnownTags.TypeParameter: return 25;
                case WellKnownTags.Delegate: return 7;
            }
        }
        return 1;
    }

    // ---- hover ---------------------------------------------------------------

    public async Task<JsonNode?> Hover(string uri, (int Line, int Character) position)
    {
        var document = DocumentFor(uri);
        var offset = document is null ? null : await Offset(document, position);
        if (document is null || offset is null)
        {
            return null;
        }
        var service = QuickInfoService.GetService(document);
        var info = service is null ? null : await service.GetQuickInfoAsync(document, offset.Value);
        if (info is null || info.Sections.IsEmpty)
        {
            return null;
        }
        var markdown = new StringBuilder();
        foreach (var section in info.Sections)
        {
            if (string.IsNullOrWhiteSpace(section.Text))
            {
                continue;
            }
            if (section.Kind == QuickInfoSectionKinds.Description)
            {
                markdown.Append("```csharp\n").Append(section.Text).Append("\n```\n\n");
            }
            else
            {
                markdown.Append(section.Text.Replace("\r\n", "\n")).Append("\n\n");
            }
        }
        var text = await document.GetTextAsync();
        return new JsonObject
        {
            ["contents"] = new JsonObject { ["kind"] = "markdown", ["value"] = markdown.ToString().TrimEnd() },
            ["range"] = Range(text.Lines.GetLinePositionSpan(info.Span)),
        };
    }

    // ---- definition ----------------------------------------------------------

    public async Task<JsonNode?> Definition(string uri, (int Line, int Character) position)
    {
        var document = DocumentFor(uri);
        var offset = document is null ? null : await Offset(document, position);
        if (document is null || offset is null)
        {
            return null;
        }
        var symbol = await SymbolFinder.FindSymbolAtPositionAsync(document, offset.Value);
        if (symbol is null)
        {
            return null;
        }
        // `numbers.Dump()` is a call to `DumpExtensions.Dump(this T, …)`.
        if (symbol is IMethodSymbol { ReducedFrom: { } unreduced })
        {
            symbol = unreduced;
        }
        symbol = symbol.OriginalDefinition;

        var locations = new JsonArray();
        foreach (var location in symbol.Locations.Where(l => l.IsInSource && l.SourceTree?.FilePath is { Length: > 0 }))
        {
            var path = location.SourceTree!.FilePath;
            var target = _open.Values.FirstOrDefault(o => o.Path == path)?.Uri ?? new Uri(path).AbsoluteUri;
            locations.Add(new JsonObject
            {
                ["uri"] = target,
                ["range"] = Range(location.GetLineSpan().Span),
            });
        }
        if (locations.Count > 0)
        {
            return locations;
        }

        // From an assembly: the type's signatures, written out as source in
        // the plugin's scratch folder and opened read-only (02-09).
        var written = MetadataSource.Write(symbol, _metadataDir);
        if (written is null)
        {
            return null;
        }
        var start = new LinePosition(written.Value.Line, 0);
        return new JsonObject
        {
            ["uri"] = new Uri(written.Value.Path).AbsoluteUri,
            ["range"] = Range(new LinePositionSpan(start, start)),
        };
    }

    // ---- warming up ----------------------------------------------------------

    /// <summary>
    /// A completion and a compile in a throwaway project: the first of each in
    /// a process is slow (the framework's metadata, the compiler's own JIT),
    /// and nobody should be typing while that happens.
    /// </summary>
    public async Task Warm()
    {
        const string uri = "file:///tinkerpad-warmup/main.cs";
        try
        {
            var source = "var numbers = new[] { 1 };\nConsole.\n";
            Update(uri, 0, source, () => { });
            await Complete(uri, (1, 8));
            await Diagnose(uri);
        }
        catch (Exception e)
        {
            _log($"could not warm up: {e.Message}");
        }
        finally
        {
            Close(uri);
        }
    }

    private static JsonObject Range(LinePositionSpan span) => new()
    {
        ["start"] = new JsonObject { ["line"] = span.Start.Line, ["character"] = span.Start.Character },
        ["end"] = new JsonObject { ["line"] = span.End.Line, ["character"] = span.End.Character },
    };
}
