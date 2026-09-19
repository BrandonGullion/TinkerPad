// `#:package Name@Version`, resolved with NuGet into the user's global
// packages folder — the same folder `dotnet restore` uses, so a package the
// machine already has is not downloaded again.
//
// What this does, for one directive: pick a version (the newest stable, if the
// directive didn't say), make sure the package is in the global folder,
// downloading it if it isn't, then take the assemblies from the `lib/` folder
// nearest to net10.0 and do the same for everything it depends on.
//
// What it deliberately doesn't do: solve version conflicts the way a real
// restore does. A playground is one file with a handful of packages; when two
// of them want different versions of the same thing, the higher one wins, which
// is what NuGet would have done for the common case.

using NuGet.Common;
using NuGet.Configuration;
using NuGet.Frameworks;
using NuGet.Packaging;
using NuGet.Packaging.Core;
using NuGet.Packaging.Signing;
using NuGet.Protocol;
using NuGet.Protocol.Core.Types;
using NuGet.Versioning;

namespace TinkerPad.CSharp.Helper;

/// <summary>Assemblies a playground's packages contribute to a compilation.</summary>
internal sealed class Packages
{
    private static readonly NuGetFramework Target = NuGetFramework.Parse("net10.0");

    private readonly ISettings _settings = Settings.LoadDefaultSettings(root: null);
    private readonly SourceCacheContext _cache = new();
    private readonly ILogger _log = NullLogger.Instance;
    private readonly string _globalFolder;
    private readonly SourceRepository _source;

    /// <summary>
    /// What a package contributes, by `id@version`, kept for the session:
    /// its own assemblies *and* its dependencies'. Caching only its own would
    /// work the first time and fail the second — a meta-package like Humanizer
    /// has no assemblies at all, and everything it offers comes from what it
    /// depends on.
    /// </summary>
    private readonly Dictionary<string, IReadOnlyList<string>> _resolved = new(StringComparer.OrdinalIgnoreCase);

    public Packages()
    {
        _globalFolder = SettingsUtility.GetGlobalPackagesFolder(_settings);
        _source = Repository.Factory.GetCoreV3("https://api.nuget.org/v3/index.json");
    }

    /// <summary>
    /// The assembly paths these directives bring in. Throws with a message
    /// worth showing when a package can't be had.
    /// </summary>
    public IReadOnlyList<string> Assemblies(
        IReadOnlyList<PackageDirective> directives,
        CancellationToken token)
    {
        var paths = new List<string>();
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var directive in directives)
        {
            Add(directive.Id, directive.Version, paths, seen, token);
        }
        return paths;
    }

    private void Add(
        string id,
        string? version,
        List<string> paths,
        HashSet<string> seen,
        CancellationToken token)
    {
        if (!seen.Add(id))
        {
            return; // Already in, at whichever version got here first.
        }

        var identity = Identify(id, version, token);
        var key = identity.ToString();
        if (_resolved.TryGetValue(key, out var cached))
        {
            paths.AddRange(cached);
            return;
        }

        // Everything this package brings, dependencies included, so the cache
        // above can hand it all back next time.
        var closure = new List<string>();
        using var reader = Install(identity, token);
        var mine = new List<string>();
        var folder = Path.Combine(_globalFolder, identity.Id.ToLowerInvariant(), identity.Version.ToNormalizedString());

        var libs = reader.GetLibItems().ToList();
        var nearest = new FrameworkReducer().GetNearest(Target, libs.Select(l => l.TargetFramework));
        if (nearest is not null)
        {
            foreach (var file in libs.First(l => l.TargetFramework.Equals(nearest)).Items)
            {
                if (file.EndsWith(".dll", StringComparison.OrdinalIgnoreCase)
                    && !file.EndsWith(".resources.dll", StringComparison.OrdinalIgnoreCase))
                {
                    mine.Add(Path.Combine(folder, file.Replace('/', Path.DirectorySeparatorChar)));
                }
            }
        }

        closure.AddRange(mine);

        // And whatever it needs to work.
        var groups = reader.GetPackageDependencies().ToList();
        var nearestDeps = new FrameworkReducer().GetNearest(Target, groups.Select(g => g.TargetFramework));
        if (nearestDeps is not null)
        {
            foreach (var dependency in groups.First(g => g.TargetFramework.Equals(nearestDeps)).Packages)
            {
                var wanted = dependency.VersionRange.MinVersion?.ToNormalizedString();
                Add(dependency.Id, wanted, closure, seen, token);
            }
        }

        _resolved[key] = closure;
        paths.AddRange(closure);
    }

    /// <summary>
    /// The nuget.org endpoint that finds and downloads packages. Asked for
    /// each time, which is a dictionary lookup after the first.
    /// </summary>
    private FindPackageByIdResource Finder() =>
        _source.GetResourceAsync<FindPackageByIdResource>().GetAwaiter().GetResult()
        ?? throw new InvalidOperationException("nuget.org is not answering");

    /// <summary>The version to use: the one asked for, or the newest stable.</summary>
    private PackageIdentity Identify(string id, string? version, CancellationToken token)
    {
        if (version is not null && NuGetVersion.TryParse(version, out var pinned))
        {
            return new PackageIdentity(id, pinned);
        }
        var versions = Finder().GetAllVersionsAsync(id, _cache, _log, token).GetAwaiter().GetResult().ToList();
        if (versions.Count == 0)
        {
            throw new InvalidOperationException($"no package called {id} on nuget.org");
        }
        var best = versions.Where(v => !v.IsPrerelease).DefaultIfEmpty(versions.Max()).Max();
        return new PackageIdentity(id, best);
    }

    /// <summary>The package in the global folder, downloaded if it isn't there.</summary>
    private PackageReaderBase Install(PackageIdentity identity, CancellationToken token)
    {
        var have = GlobalPackagesFolderUtility.GetPackage(identity, _globalFolder);
        if (have?.PackageReader is not null)
        {
            return have.PackageReader;
        }

        using var stream = new MemoryStream();
        var copied = Finder()
            .CopyNupkgToStreamAsync(identity.Id, identity.Version, stream, _cache, _log, token)
            .GetAwaiter()
            .GetResult();
        if (!copied)
        {
            throw new InvalidOperationException(
                $"couldn't download {identity.Id} {identity.Version} from nuget.org");
        }
        stream.Position = 0;
        var added = GlobalPackagesFolderUtility
            .AddPackageAsync(
                _source.PackageSource.Source,
                identity,
                stream,
                _globalFolder,
                parentId: Guid.Empty,
                ClientPolicyContext.GetClientPolicy(_settings, _log),
                _log,
                token)
            .GetAwaiter()
            .GetResult();
        return added.PackageReader
            ?? throw new InvalidOperationException($"couldn't unpack {identity.Id} {identity.Version}");
    }
}
