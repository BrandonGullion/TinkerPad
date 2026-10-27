// Go to definition on something from an assembly (work item 04-13).
//
// There is no source to go to, and Roslyn's own "metadata as source" isn't
// public API. What it shows is mostly signatures anyway, so this writes those:
// the type, its public and protected members, and the first line of each one's
// documentation, as a `.cs` file in the plugin's scratch folder. The editor
// opens it read-only, like any definition outside the playground (02-09).
// Not decompiled bodies: that would be a decompiler, and a dependency, for
// something a signature usually answers.

using System.Text;
using System.Xml.Linq;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

namespace TinkerPad.CSharp.Helper;

internal static class MetadataSource
{
    private static readonly SymbolDisplayFormat Signature = new(
        globalNamespaceStyle: SymbolDisplayGlobalNamespaceStyle.Omitted,
        typeQualificationStyle: SymbolDisplayTypeQualificationStyle.NameAndContainingTypes,
        genericsOptions: SymbolDisplayGenericsOptions.IncludeTypeParameters
            | SymbolDisplayGenericsOptions.IncludeTypeConstraints
            | SymbolDisplayGenericsOptions.IncludeVariance,
        memberOptions: SymbolDisplayMemberOptions.IncludeAccessibility
            | SymbolDisplayMemberOptions.IncludeModifiers
            | SymbolDisplayMemberOptions.IncludeParameters
            | SymbolDisplayMemberOptions.IncludeType
            | SymbolDisplayMemberOptions.IncludeRef
            | SymbolDisplayMemberOptions.IncludeConstantValue,
        parameterOptions: SymbolDisplayParameterOptions.IncludeType
            | SymbolDisplayParameterOptions.IncludeName
            | SymbolDisplayParameterOptions.IncludeParamsRefOut
            | SymbolDisplayParameterOptions.IncludeDefaultValue
            | SymbolDisplayParameterOptions.IncludeExtensionThis,
        propertyStyle: SymbolDisplayPropertyStyle.ShowReadWriteDescriptor,
        kindOptions: SymbolDisplayKindOptions.IncludeTypeKeyword,
        miscellaneousOptions: SymbolDisplayMiscellaneousOptions.UseSpecialTypes
            | SymbolDisplayMiscellaneousOptions.EscapeKeywordIdentifiers
            | SymbolDisplayMiscellaneousOptions.IncludeNullableReferenceTypeModifier);

    /// <summary>
    /// Write the outermost type `symbol` belongs to, and say which line
    /// (from zero) `symbol` is on. Null for something that has no type, such
    /// as a namespace.
    /// </summary>
    public static (string Path, int Line)? Write(ISymbol symbol, string directory)
    {
        var type = symbol as INamedTypeSymbol ?? symbol.ContainingType;
        if (type is null)
        {
            return null;
        }
        while (type.ContainingType is not null)
        {
            type = type.ContainingType;
        }

        var lines = new List<string>
        {
            $"// {type.ContainingAssembly?.Identity.GetDisplayName() ?? "metadata"}",
            "// Signatures only, written by TinkerPad from the assembly's metadata.",
            "",
        };
        var target = 0;
        if (!type.ContainingNamespace.IsGlobalNamespace)
        {
            lines.Add($"namespace {type.ContainingNamespace.ToDisplayString()};");
            lines.Add("");
        }
        WriteType(type, symbol, "", lines, ref target);

        var assembly = type.ContainingAssembly?.Name ?? "metadata";
        var path = Path.Combine(directory, Safe(assembly), Safe(type.ToDisplayString()) + ".cs");
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path, string.Join("\n", lines) + "\n", new UTF8Encoding(false));
        return (path, target);
    }

    private static void WriteType(INamedTypeSymbol type, ISymbol wanted, string indent, List<string> lines, ref int target)
    {
        Summary(type, indent, lines);
        if (SymbolEqualityComparer.Default.Equals(type, wanted))
        {
            target = lines.Count;
        }
        lines.Add(indent + Declaration(type));
        lines.Add(indent + "{");
        foreach (var member in type.GetMembers().Where(Shown))
        {
            if (member is INamedTypeSymbol nested)
            {
                WriteType(nested, wanted, indent + "    ", lines, ref target);
                continue;
            }
            Summary(member, indent + "    ", lines);
            if (SymbolEqualityComparer.Default.Equals(member, wanted))
            {
                target = lines.Count;
            }
            var line = member.ToDisplayString(Signature);
            lines.Add(indent + "    " + line + (member is IPropertySymbol ? "" : ";"));
        }
        lines.Add(indent + "}");
        lines.Add("");
    }

    /// <summary>`public static class Console`, with what it derives from.</summary>
    private static string Declaration(INamedTypeSymbol type)
    {
        var parts = new List<string> { SyntaxFacts.GetText(type.DeclaredAccessibility) };
        if (type.TypeKind == TypeKind.Class)
        {
            if (type.IsStatic)
            {
                parts.Add("static");
            }
            else if (type.IsAbstract)
            {
                parts.Add("abstract");
            }
            else if (type.IsSealed)
            {
                parts.Add("sealed");
            }
        }
        parts.Add(type.ToDisplayString(Signature));
        var bases = new List<string>();
        if (type.BaseType is { SpecialType: not (SpecialType.System_Object or SpecialType.System_ValueType or SpecialType.System_Enum or SpecialType.System_MulticastDelegate) } baseType
            && type.TypeKind == TypeKind.Class)
        {
            bases.Add(baseType.ToDisplayString(Signature));
        }
        bases.AddRange(type.Interfaces.Select(i => i.ToDisplayString(Signature)));
        return string.Join(" ", parts.Where(p => p.Length > 0)) + (bases.Count > 0 ? " : " + string.Join(", ", bases) : "");
    }

    /// <summary>What someone using the type can see.</summary>
    private static bool Shown(ISymbol member)
    {
        if (member.IsImplicitlyDeclared
            || member.DeclaredAccessibility is not (Accessibility.Public or Accessibility.Protected or Accessibility.ProtectedOrInternal))
        {
            return false;
        }
        return member switch
        {
            IMethodSymbol method => method.MethodKind is MethodKind.Ordinary or MethodKind.Constructor
                or MethodKind.UserDefinedOperator or MethodKind.Conversion,
            _ => true,
        };
    }

    /// <summary>The first sentence or so of a symbol's documentation, as a comment.</summary>
    private static void Summary(ISymbol symbol, string indent, List<string> lines)
    {
        var xml = symbol.GetDocumentationCommentXml();
        if (string.IsNullOrWhiteSpace(xml))
        {
            return;
        }
        try
        {
            var summary = XElement.Parse(xml).Element("summary");
            var text = summary is null ? null : string.Join(" ", summary.DescendantNodes()
                .Select(node => node switch
                {
                    XText t => t.Value,
                    XElement { Name.LocalName: "see" or "paramref" or "typeparamref" } e =>
                        (string?)e.Attribute("cref") ?? (string?)e.Attribute("name") ?? (string?)e.Attribute("langword") ?? "",
                    _ => "",
                }))
                .Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);
            if (text is { Length: > 0 })
            {
                lines.Add($"{indent}/// {string.Join(" ", text).Replace("T:", "").Replace("M:", "").Replace("P:", "")}");
            }
        }
        catch (System.Xml.XmlException)
        {
            // Documentation that isn't XML is documentation we can skip.
        }
    }

    private static string Safe(string name)
    {
        var invalid = Path.GetInvalidFileNameChars();
        return new string(name.Select(c => invalid.Contains(c) || c is '<' or '>' ? '_' : c).ToArray());
    }
}
