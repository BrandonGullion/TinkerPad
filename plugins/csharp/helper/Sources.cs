// Reading a playground's file the way the compiler needs it.
//
// A file-based C# app may start with directives the language doesn't know:
//
//     #:package Humanizer@2.14.1
//     #:sdk Microsoft.NET.Sdk
//
// `dotnet run --file` reads those and strips them before the compiler sees
// them. So does this — but by *blanking* each line rather than removing it, so
// every line number below stays where the editor put it, and a diagnostic on
// line 12 is line 12 in the tab.

using System.Text;
using Microsoft.CodeAnalysis.Text;

namespace TinkerPad.CSharp.Helper;

/// <summary>One `#:package Name@Version` from the top of a playground.</summary>
internal readonly record struct PackageDirective(string Id, string? Version);

internal static class Sources
{
    /// <summary>
    /// The playground's text, ready for Roslyn, and whatever it asked for.
    /// Read as bytes so the encoding is the file's own, which is what a
    /// portable PDB records (docs/DEBUGGING.md D9).
    /// </summary>
    public static SourceText Read(string path, out IReadOnlyList<PackageDirective> directives)
    {
        using var file = File.OpenRead(path);
        var text = SourceText.From(file, Encoding.UTF8, canBeEmbedded: true);
        var found = new List<PackageDirective>();
        var blanked = new StringBuilder(text.Length);

        foreach (var line in text.Lines)
        {
            var content = line.ToString();
            if (content.TrimStart().StartsWith("#:", StringComparison.Ordinal))
            {
                var directive = Parse(content.Trim());
                if (directive is { } package)
                {
                    found.Add(package);
                }
                // Same line, no content: positions below are unchanged.
                blanked.Append(new string(' ', 0));
            }
            else
            {
                blanked.Append(content);
            }
            blanked.Append('\n');
        }

        directives = found;
        return SourceText.From(blanked.ToString(), Encoding.UTF8);
    }

    /// <summary>`#:package Name@Version`, or nothing we act on.</summary>
    private static PackageDirective? Parse(string line)
    {
        const string prefix = "#:package";
        if (!line.StartsWith(prefix, StringComparison.Ordinal))
        {
            return null;
        }
        var rest = line[prefix.Length..].Trim();
        if (rest.Length == 0)
        {
            return null;
        }
        var at = rest.IndexOf('@');
        return at < 0
            ? new PackageDirective(rest, null)
            : new PackageDirective(rest[..at].Trim(), rest[(at + 1)..].Trim());
    }
}
