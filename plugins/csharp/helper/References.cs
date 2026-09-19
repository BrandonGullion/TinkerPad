// What a playground is compiled against: the .NET it is running on, plus
// whatever its `#:package` directives ask for.
//
// The framework set is read once and kept: it is ~200 assemblies, and reading
// their metadata is most of what a cold compile costs. Package assemblies are
// kept per (name, version) for the life of the helper, so the second run that
// uses Humanizer doesn't touch the disk, let alone the network.

using Microsoft.CodeAnalysis;

namespace TinkerPad.CSharp.Helper;

internal sealed class References
{
    private readonly Core _core;
    private readonly Lazy<IReadOnlyList<MetadataReference>> _framework;
    private readonly Lazy<Packages> _packages = new(() => new Packages());
    private readonly Dictionary<string, MetadataReference> _byPath =
        new(StringComparer.OrdinalIgnoreCase);

    public References(Core core)
    {
        _core = core;
        _framework = new Lazy<IReadOnlyList<MetadataReference>>(LoadFramework);
    }

    /// <summary>The .NET a playground is compiled against, on its own.</summary>
    public IReadOnlyList<MetadataReference> Framework => _framework.Value;

    /// <summary>
    /// Everything to compile against. `problem` is set when a package couldn't
    /// be had, which is a compile error with a sentence worth reading rather
    /// than a crash.
    /// </summary>
    public IReadOnlyList<MetadataReference> For(
        IReadOnlyList<PackageDirective> directives,
        CancellationToken token,
        out IReadOnlyList<string> packages,
        out string? problem)
    {
        problem = null;
        packages = [];
        var references = new List<MetadataReference>(_framework.Value);
        if (directives.Count == 0)
        {
            return references;
        }

        try
        {
            var paths = _packages.Value.Assemblies(directives, token);
            packages = paths;
            foreach (var path in paths)
            {
                var reference = Reference(path);
                if (reference is not null)
                {
                    references.Add(reference);
                }
            }
        }
        catch (Exception e)
        {
            problem = $"TP0001: {e.Message}";
            Core.Warn($"packages: {e}");
        }
        return references;
    }

    /// <summary>
    /// The assemblies of the .NET this helper is running on — which is the
    /// same .NET a per-run `dotnet run` would give the playground, since both
    /// come from the SDK the plugin's toolchain probe found.
    /// </summary>
    private IReadOnlyList<MetadataReference> LoadFramework()
    {
        var references = new List<MetadataReference>();
        var directory = Path.GetDirectoryName(typeof(object).Assembly.Location);
        if (directory is not null)
        {
            foreach (var file in Directory.EnumerateFiles(directory, "*.dll"))
            {
                // The shared framework folder holds native libraries too —
                // `clrjit`, `System.IO.Compression.Native` — and the compiler
                // only finds out they aren't assemblies when it emits. Asking
                // for the assembly name reads the header and says so here.
                if (!IsManaged(file))
                {
                    continue;
                }
                var reference = Reference(file);
                if (reference is not null)
                {
                    references.Add(reference);
                }
            }
        }
        if (references.Count == 0)
        {
            Core.Warn("no framework assemblies found; playgrounds will not compile");
        }
        return references;
    }

    /// <summary>Whether a file is a .NET assembly rather than a native DLL.</summary>
    private static bool IsManaged(string path)
    {
        try
        {
            System.Reflection.AssemblyName.GetAssemblyName(path);
            return true;
        }
        catch (BadImageFormatException)
        {
            return false;
        }
        catch (Exception e)
        {
            Core.Warn($"skipping {Path.GetFileName(path)}: {e.Message}");
            return false;
        }
    }

    /// <summary>One assembly's metadata, read once and shared.</summary>
    private MetadataReference? Reference(string path)
    {
        if (_byPath.TryGetValue(path, out var known))
        {
            return known;
        }
        try
        {
            var reference = MetadataReference.CreateFromFile(path);
            _byPath[path] = reference;
            return reference;
        }
        catch (Exception e)
        {
            // A native DLL, or one being written: not worth stopping for.
            Core.Warn($"skipping {Path.GetFileName(path)}: {e.Message}");
            return null;
        }
    }
}
