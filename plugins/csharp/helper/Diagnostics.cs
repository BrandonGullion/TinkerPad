// Roslyn's diagnostics, in the shape the protocol asks for (docs/PLUGINS.md §5).
//
// Positions go straight through: the helper compiles the file on disk as it is
// written, locked lines and all, so line 7 here is line 7 in the editor and
// nothing has to be offset.

using System.Text.Json.Nodes;
using Microsoft.CodeAnalysis;

namespace TinkerPad.CSharp.Helper;

internal static class Diagnostics
{
    public static JsonObject From(Diagnostic diagnostic)
    {
        var span = diagnostic.Location.GetLineSpan();
        var severity = diagnostic.Severity switch
        {
            DiagnosticSeverity.Error => "error",
            DiagnosticSeverity.Warning => "warning",
            _ => "info",
        };
        return Of(
            diagnostic.Location.SourceTree?.FilePath ?? span.Path,
            // Roslyn counts from zero; everyone reading this counts from one.
            span.StartLinePosition.Line + 1,
            span.StartLinePosition.Character + 1,
            $"{diagnostic.Id}: {diagnostic.GetMessage()}",
            severity);
    }

    public static JsonObject Of(string? file, int line, int column, string message, string severity) =>
        new()
        {
            ["file"] = file,
            ["line"] = line,
            ["col"] = column,
            ["message"] = message,
            ["severity"] = severity,
        };

    /// <summary>What the results pane shows above the squiggles.</summary>
    public static string Summarise(IEnumerable<JsonObject> diagnostics)
    {
        var errors = diagnostics
            .Where(d => d["severity"]?.GetValue<string>() == "error")
            .Select(d => $"{d["message"]?.GetValue<string>()}")
            .ToList();
        return errors.Count switch
        {
            0 => "The code didn't compile.",
            1 => errors[0],
            _ => $"{errors[0]} (and {errors.Count - 1} more)",
        };
    }
}
