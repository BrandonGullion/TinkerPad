// Compiling and running one playground, in this process.
//
// Roslyn stays warm between runs, which is the whole point: the first compile
// of a session pays for the compiler's own start-up, and every one after it is
// a parse, a compile, and an assembly load. Each run gets its own collectible
// `AssemblyLoadContext`, so types and statics from the last run are gone.
//
// The file on disk is compiled as it is written, with `path:` set to it and
// portable PDBs emitted, so a stack trace points at `main.cs:line N` and a
// debugger can find it later (docs/DEBUGGING.md D9).

using System.Diagnostics;
using System.Runtime.CompilerServices;
using System.Reflection;
using System.Runtime.Loader;
using System.Text;
using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Emit;
using Microsoft.CodeAnalysis.Text;

namespace TinkerPad.CSharp.Helper;

internal sealed class Runner
{
    /// <summary>The thread every playground runs on: named so a stack trace,
    /// a profiler, and a debugger all agree about which one it is.</summary>
    private const string RunThread = "tinkerpad-run";

    /// <summary>
    /// What `&lt;ImplicitUsings&gt;enable&lt;/ImplicitUsings&gt;` means for a
    /// console app — which is what a file-based C# app is. Without these,
    /// `Console.WriteLine` in a playground doesn't compile here but does under
    /// `dotnet run`, and the two paths have to agree.
    /// </summary>
    internal const string ImplicitUsings = """
        global using global::System;
        global using global::System.Collections.Generic;
        global using global::System.IO;
        global using global::System.Linq;
        global using global::System.Net.Http;
        global using global::System.Threading;
        global using global::System.Threading.Tasks;
        """;

    internal static readonly CSharpParseOptions Parsing = new(LanguageVersion.Preview);

    /// <summary>
    /// How a playground is compiled. The language server compiles with the same
    /// options, so a squiggle and a run can't disagree (work item 04-13).
    /// </summary>
    internal static readonly CSharpCompilationOptions Options = new(
        OutputKind.ConsoleApplication,
        optimizationLevel: OptimizationLevel.Debug,
        // A playground is one file; nothing should be forced into a
        // namespace or told off for lacking an XML comment.
        warningLevel: 4,
        allowUnsafe: true);

    private readonly Core _core;
    private readonly References _references;
    private readonly string _dumpLibrary;

    /// <summary>
    /// The trees that are the same for every run: the implicit usings, and the
    /// dump library. Parsed once — the dump library is six hundred lines, and
    /// parsing it again each time was a fifth of what a run cost.
    /// </summary>
    private readonly Lazy<IReadOnlyList<SyntaxTree>> _fixedTrees;

    /// <summary>The compiled dump library, once there is one.</summary>
    private MetadataReference? _dumpAssembly;

    /// <summary>What is running, if anything. One at a time (§5).</summary>
    private Run? _current;

    public Runner(Core core, string pluginDir)
    {
        _core = core;
        _references = new References();
        _dumpLibrary = Path.Combine(pluginDir, "dump", "TinkerPad.Dump.cs");
        _fixedTrees = new Lazy<IReadOnlyList<SyntaxTree>>(ParseFixedTrees);
    }

    private IReadOnlyList<SyntaxTree> ParseFixedTrees()
    {
        var trees = new List<SyntaxTree>
        {
            CSharpSyntaxTree.ParseText(
                // With an encoding: a tree without one can't carry debug info,
                // and every tree in a compilation that emits a PDB needs it
                // (CS8055).
                SourceText.From(ImplicitUsings, Encoding.UTF8),
                Parsing,
                path: "GlobalUsings.g.cs"),
        };
        // The dump library is compiled once into an assembly of its own (see
        // `BuildDumpLibrary`). Only if that fails does its source come along
        // for the ride, which costs every run the time to bind six hundred
        // lines it already bound last time.
        if (_dumpAssembly is null && File.Exists(_dumpLibrary))
        {
            using var file = File.OpenRead(_dumpLibrary);
            trees.Add(CSharpSyntaxTree.ParseText(
                SourceText.From(file, Encoding.UTF8),
                Parsing,
                path: _dumpLibrary));
        }
        return trees;
    }

    /// <summary>
    /// Compile the plugin's `Dump()` library once, into an assembly every run
    /// references and shares.
    /// </summary>
    /// <remarks>
    /// The per-run path compiles this file into the playground itself. Here it
    /// would be compiled again for every run, which is most of what a run
    /// costs; as an assembly it is one metadata reference. It is loaded into
    /// the default context, not a run's, so all of them share the one copy —
    /// which is also why `Dump()` keeps working after a run is unloaded.
    /// </remarks>
    private void BuildDumpLibrary()
    {
        if (!File.Exists(_dumpLibrary))
        {
            Core.Warn($"no dump library at {_dumpLibrary}; Dump() will not compile");
            return;
        }
        try
        {
            using var file = File.OpenRead(_dumpLibrary);
            var tree = CSharpSyntaxTree.ParseText(
                SourceText.From(file, Encoding.UTF8),
                Parsing,
                path: _dumpLibrary);
            var compilation = CSharpCompilation.Create(
                "TinkerPad.Dump",
                [tree],
                _references.Framework,
                new CSharpCompilationOptions(
                    OutputKind.DynamicallyLinkedLibrary,
                    optimizationLevel: OptimizationLevel.Release));

            using var dll = new MemoryStream();
            var result = compilation.Emit(dll);
            if (!result.Success)
            {
                var first = result.Diagnostics.FirstOrDefault(d => d.Severity == DiagnosticSeverity.Error);
                Core.Warn($"the dump library didn't compile ({first?.GetMessage()}); using its source instead");
                return;
            }
            var bytes = dll.ToArray();
            _dumpAssembly = MetadataReference.CreateFromImage(bytes);
            AssemblyLoadContext.Default.LoadFromStream(new MemoryStream(bytes));
        }
        catch (Exception e)
        {
            Core.Warn($"could not build the dump library ({e.Message}); using its source instead");
            _dumpAssembly = null;
        }
    }

    /// <summary>Start a playground. Returns once it is going, not once it is done.</summary>
    public void Start(string runId, string mainPath, string workingDirectory)
    {
        var run = new Run(runId, mainPath, workingDirectory);
        _current = run;
        var thread = new Thread(() => Execute(run), 16 * 1024 * 1024)
        {
            Name = RunThread,
            IsBackground = true,
        };
        run.Thread = thread;
        thread.Start();
    }

    /// <summary>
    /// Compile something and throw the result away, before anyone is waiting.
    /// </summary>
    /// <remarks>
    /// Roslyn's first compile in a process is the expensive one: the framework
    /// assemblies are read and the compiler's own code is jitted. Paying for
    /// it here, while the core waits for `ready`, is the difference between a
    /// first run of a second and one of a tenth of that.
    /// </remarks>
    public void Prepare(string mainPath)
    {
        BuildDumpLibrary();
        var run = new Run("warmup", mainPath, Path.GetDirectoryName(mainPath) ?? ".");
        Compile(run, out _);
    }

    /// <summary>
    /// Stop the running playground.
    /// </summary>
    /// <remarks>
    /// .NET has no way to stop a thread from outside, so this is what there
    /// is: interrupt it, which breaks any blocking wait it is in. A tight loop
    /// won't notice, and the core kills this process two seconds later —
    /// which is what that grace in the protocol is for.
    /// </remarks>
    /// <summary>What the user typed, for the program to read (04-05).</summary>
    public void Typed(string runId, string text)
    {
        var run = _current;
        if (run is not null && run.Id == runId)
        {
            run.Input?.Feed(text);
        }
    }

    /// <summary>The user closed the input: reads return end of input.</summary>
    public void InputEnded(string runId)
    {
        var run = _current;
        if (run is not null && run.Id == runId)
        {
            run.Input?.End();
        }
    }

    public void Cancel(string runId)
    {
        var run = _current;
        if (run is null || run.Id != runId)
        {
            return;
        }
        // A cancelled program is not going to read anything else.
        run.Input?.End();
        run.Cancelled = true;
        run.Cancellation.Cancel();
        try
        {
            run.Thread?.Interrupt();
        }
        catch (Exception e)
        {
            Core.Warn($"could not interrupt the run: {e.Message}");
        }
        run.Thread?.Join(TimeSpan.FromSeconds(1.5));
        if (_current == run)
        {
            _current = null;
            _core.Exited(runId, null, run.Elapsed);
        }
    }

    /// <summary>
    /// Run it, then let its assemblies go.
    /// </summary>
    /// <remarks>
    /// The two halves are separate methods on purpose: nothing may hold a
    /// reference to the load context while the collector is asked to take it,
    /// and a local variable counts as holding one. This is the shape the .NET
    /// documentation gives for unloadability, and getting it wrong means every
    /// run looks like a leak.
    /// </remarks>
    private void Execute(Run run)
    {
        var reference = ExecuteInContext(run);
        Collect(reference);
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private WeakReference ExecuteInContext(Run run)
    {
        var context = new RunContext(run.Id);
        try
        {
            var compiled = Compile(run, out var diagnostics);
            if (compiled is null)
            {
                _core.Failed(run.Id, Diagnostics.Summarise(diagnostics), diagnostics);
                _current = null;
                return new WeakReference(context);
            }

            var (dll, pdb, packages) = compiled.Value;
            // The packages this playground asked for, so `using Humanizer;`
            // still means something once it is running and not just while it
            // compiles.
            context.Offer(packages);
            // LoadFromStream with the PDB: this is what puts `main.cs:line N`
            // in a stack trace (docs/DEBUGGING.md D9).
            var assembly = context.LoadFromStream(new MemoryStream(dll), new MemoryStream(pdb));
            Invoke(run, assembly);
        }
        catch (Exception e)
        {
            _core.Output("stderr", run.Id, $"The helper could not run this: {e}\n");
            Finish(run, 1);
        }
        finally
        {
            try
            {
                context.Unload();
            }
            catch (Exception e)
            {
                Core.Warn($"could not unload the run: {e.Message}");
            }
        }
        return new WeakReference(context);
    }

    /// <summary>Compile the playground, with the dump library alongside it.</summary>
    private (byte[] Dll, byte[] Pdb, IReadOnlyList<string> Packages)? Compile(
        Run run,
        out List<JsonObject> diagnostics)
    {
        diagnostics = [];
        var source = Sources.Read(run.MainPath, out var directives);
        // The playground itself, parsed with its own path so a stack trace and
        // a diagnostic both name the file the editor is showing.
        var trees = new List<SyntaxTree>(_fixedTrees.Value.Count + 1)
        {
            CSharpSyntaxTree.ParseText(source, Parsing, path: run.MainPath),
        };
        trees.AddRange(_fixedTrees.Value);

        var references = _references.For(
            directives,
            run.Cancellation.Token,
            out var packages,
            out var packageProblem);
        if (packageProblem is not null)
        {
            diagnostics.Add(Diagnostics.Of(run.MainPath, 1, 1, packageProblem, "error"));
            return null;
        }

        if (_dumpAssembly is not null)
        {
            references = [.. references, _dumpAssembly];
        }

        var compilation = CSharpCompilation.Create(
            $"playground_{run.Id}",
            trees,
            references,
            Options);

        using var dll = new MemoryStream();
        using var pdb = new MemoryStream();
        var result = compilation.Emit(
            dll,
            pdb,
            options: new EmitOptions(debugInformationFormat: DebugInformationFormat.PortablePdb));

        foreach (var diagnostic in result.Diagnostics)
        {
            if (diagnostic.Severity is DiagnosticSeverity.Error or DiagnosticSeverity.Warning)
            {
                diagnostics.Add(Diagnostics.From(diagnostic));
            }
        }
        if (!result.Success)
        {
            return null;
        }
        // Warnings are worth seeing, but they are not why a run stopped, so
        // they go to stderr rather than into `failed`.
        foreach (var warning in result.Diagnostics.Where(d => d.Severity == DiagnosticSeverity.Warning))
        {
            _core.Output("stderr", run.Id, warning.ToString() + "\n");
        }
        diagnostics.Clear();
        return (dll.ToArray(), pdb.ToArray(), packages);
    }

    /// <summary>Run the compiled entry point with the console pointed at us.</summary>
    private void Invoke(Run run, Assembly assembly)
    {
        var entry = assembly.EntryPoint;
        if (entry is null)
        {
            _core.Output("stderr", run.Id, "This playground has no entry point.\n");
            Finish(run, 1);
            return;
        }

        var stdout = new ConsoleStream(_core, "stdout", run.Id);
        var stderr = new ConsoleStream(_core, "stderr", run.Id);
        var wasOut = Console.Out;
        var wasError = Console.Error;
        var wasDirectory = Directory.GetCurrentDirectory();
        Console.SetOut(stdout);
        Console.SetError(stderr);
        // Console.ReadLine() waits for what the user types in TinkerPad
        // (work item 04-05), and reads end of input when they close it.
        var stdin = new ConsoleInput();
        run.Input = stdin;
        Console.SetIn(stdin);
        try
        {
            Directory.SetCurrentDirectory(run.WorkingDirectory);
        }
        catch (Exception e)
        {
            Core.Warn($"could not enter {run.WorkingDirectory}: {e.Message}");
        }

        int code = 0;
        try
        {
            var arguments = entry.GetParameters().Length == 0
                ? null
                : new object?[] { Array.Empty<string>() };
            var returned = entry.Invoke(null, arguments);
            code = returned switch
            {
                int value => value,
                Task<int> task => task.GetAwaiter().GetResult(),
                Task task => Await(task),
                _ => 0,
            };
        }
        catch (TargetInvocationException wrapped) when (wrapped.InnerException is not null)
        {
            code = Report(run, wrapped.InnerException);
        }
        catch (Exception e)
        {
            code = Report(run, e);
        }
        finally
        {
            stdout.Flush();
            stderr.Flush();
            Console.SetOut(wasOut);
            Console.SetError(wasError);
            try
            {
                Directory.SetCurrentDirectory(wasDirectory);
            }
            catch (Exception e)
            {
                Core.Warn($"could not return to {wasDirectory}: {e.Message}");
            }
        }
        Finish(run, code);
    }

    private static int Await(Task task)
    {
        task.GetAwaiter().GetResult();
        return 0;
    }

    /// <summary>
    /// An exception the playground didn't catch, printed the way .NET prints
    /// it, with the exit code .NET would have given.
    /// </summary>
    private int Report(Run run, Exception e)
    {
        if (run.Cancelled && e is ThreadInterruptedException or OperationCanceledException)
        {
            return 0; // The cancel reports the ending itself.
        }
        _core.Output("stderr", run.Id, $"Unhandled exception. {Trace(e)}\n");
        return UnhandledExceptionCode;
    }

    /// <summary>
    /// What `dotnet run` would have exited with: the CLR's unhandled-exception
    /// code on Windows, and 134 (SIGABRT) elsewhere. A playground shouldn't
    /// report a different number for the same crash depending on how it ran.
    /// </summary>
    private static int UnhandledExceptionCode =>
        OperatingSystem.IsWindows() ? unchecked((int)0xE0434352) : 134;

    /// <summary>
    /// The exception as the user would have seen it, without the helper's own
    /// frames. Reflection is how the entry point is called here; it isn't part
    /// of their program, and `dotnet run` doesn't show it.
    /// </summary>
    private static string Trace(Exception e)
    {
        var text = e.ToString();
        var cut = text.IndexOf("\n   at System.Reflection.", StringComparison.Ordinal);
        return cut < 0 ? text : text[..cut];
    }

    private void Finish(Run run, int code)
    {
        if (run.Cancelled)
        {
            return;
        }
        if (_current == run)
        {
            _current = null;
        }
        _core.Exited(run.Id, code, run.Elapsed);
    }

    /// <summary>
    /// Wait for the run's assemblies to go. A playground that left a thread or
    /// a timer running keeps its context alive; that is worth saying, because
    /// the next runs share this process with it until the helper is restarted.
    /// </summary>
    private static void Collect(WeakReference reference)
    {
        for (var attempt = 0; attempt < 10 && reference.IsAlive; attempt++)
        {
            GC.Collect();
            GC.WaitForPendingFinalizers();
        }
        if (reference.IsAlive)
        {
            Core.Warn("a run is still holding memory: something it started is still going");
        }
    }

    /// <summary>
    /// The load context one run lives in: collectible, so it goes when the run
    /// does, and able to find the assemblies the run's packages brought.
    /// </summary>
    /// <remarks>
    /// Without the `Load` override a playground compiles against Humanizer and
    /// then can't find it at runtime, because nothing told the runtime where it
    /// is. The framework itself is not offered here on purpose: those come from
    /// the default context, so a run shares one copy of `System.Runtime` with
    /// the helper rather than loading its own.
    /// </remarks>
    private sealed class RunContext(string runId)
        : AssemblyLoadContext($"tinkerpad-run-{runId}", isCollectible: true)
    {
        private readonly Dictionary<string, string> _byName = new(StringComparer.OrdinalIgnoreCase);

        public void Offer(IReadOnlyList<string> assemblyPaths)
        {
            foreach (var path in assemblyPaths)
            {
                _byName[Path.GetFileNameWithoutExtension(path)] = path;
            }
        }

        protected override Assembly? Load(AssemblyName assemblyName)
        {
            if (assemblyName.Name is { } name
                && _byName.TryGetValue(name, out var path)
                && File.Exists(path))
            {
                return LoadFromAssemblyPath(path);
            }
            return null; // The default context has it, or nobody does.
        }
    }

    /// <summary>One playground, while it runs.</summary>
    private sealed class Run(string id, string mainPath, string workingDirectory)
    {
        private readonly Stopwatch _clock = Stopwatch.StartNew();

        public string Id { get; } = id;
        public string MainPath { get; } = mainPath;
        public string WorkingDirectory { get; } = workingDirectory;
        public Thread? Thread { get; set; }
        public bool Cancelled { get; set; }
        /// <summary>Where what the user types goes (work item 04-05).</summary>
        public ConsoleInput? Input { get; set; }
        public CancellationTokenSource Cancellation { get; } = new();
        public long Elapsed => _clock.ElapsedMilliseconds;
    }
}

/// <summary>
/// The playground's standard input: what the user types in TinkerPad, handed
/// over a line at a time (work item 04-05).
///
/// The protocol thread feeds it and the run's thread reads it, so everything
/// here is under one lock, and a reader waits rather than spinning.
/// </summary>
internal sealed class ConsoleInput : TextReader
{
    private readonly object _lock = new();
    private readonly StringBuilder _buffer = new();
    private bool _ended;

    /// <summary>What the user typed, from the protocol thread.</summary>
    public void Feed(string text)
    {
        lock (_lock)
        {
            _buffer.Append(text);
            Monitor.PulseAll(_lock);
        }
    }

    /// <summary>No more input is coming: readers stop waiting.</summary>
    public void End()
    {
        lock (_lock)
        {
            _ended = true;
            Monitor.PulseAll(_lock);
        }
    }

    public override string? ReadLine()
    {
        lock (_lock)
        {
            while (true)
            {
                var end = Find('\n');
                if (end >= 0)
                {
                    var line = _buffer.ToString(0, end);
                    _buffer.Remove(0, end + 1);
                    return line.EndsWith('\r') ? line[..^1] : line;
                }
                if (_ended)
                {
                    if (_buffer.Length == 0)
                    {
                        return null;
                    }
                    var rest = _buffer.ToString();
                    _buffer.Clear();
                    return rest;
                }
                Monitor.Wait(_lock);
            }
        }
    }

    public override int Read()
    {
        lock (_lock)
        {
            while (_buffer.Length == 0 && !_ended)
            {
                Monitor.Wait(_lock);
            }
            if (_buffer.Length == 0)
            {
                return -1;
            }
            var c = _buffer[0];
            _buffer.Remove(0, 1);
            return c;
        }
    }

    public override int Peek()
    {
        lock (_lock)
        {
            while (_buffer.Length == 0 && !_ended)
            {
                Monitor.Wait(_lock);
            }
            return _buffer.Length == 0 ? -1 : _buffer[0];
        }
    }

    public override string ReadToEnd()
    {
        lock (_lock)
        {
            while (!_ended)
            {
                Monitor.Wait(_lock);
            }
            var all = _buffer.ToString();
            _buffer.Clear();
            return all;
        }
    }

    private int Find(char wanted)
    {
        for (var i = 0; i < _buffer.Length; i++)
        {
            if (_buffer[i] == wanted)
            {
                return i;
            }
        }
        return -1;
    }
}

/// <summary>
/// `Console.Out` and `Console.Error` during a run: every write becomes a
/// protocol message, flushed as it happens so output appears while the
/// playground is still going.
/// </summary>
internal sealed class ConsoleStream(Core core, string kind, string runId) : TextWriter
{
    public override Encoding Encoding => Encoding.UTF8;

    public override void Write(char value) => core.Output(kind, runId, value.ToString());

    public override void Write(string? value)
    {
        if (!string.IsNullOrEmpty(value))
        {
            core.Output(kind, runId, value);
        }
    }

    public override void Write(char[] buffer, int index, int count)
    {
        if (count > 0)
        {
            core.Output(kind, runId, new string(buffer, index, count));
        }
    }

    public override void WriteLine(string? value) => Write((value ?? "") + Environment.NewLine);

    public override void WriteLine() => Write(Environment.NewLine);
}
