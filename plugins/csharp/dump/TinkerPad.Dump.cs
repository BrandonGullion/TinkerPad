// TinkerPad's dump library for C#. Compiled into every C# playground (see
// ../templates/Directory.Build.props) and kept in one file on purpose.
//
// `value.Dump(title)` or `Dump.Show(value, title)` writes one tinkerpad-dump-v1
// value to stdout as ESC ] 1337 ; TinkerPad= <json> BEL (docs/PLUGINS.md §6)
// and returns `value`. This file decides the shape; the app only renders it:
//
// - a sequence whose items are all objects with members -> table, columns
//   from the union of their public properties and fields, at most MaxRows rows;
// - any other object, sequence, or dictionary              -> tree;
// - everything else (numbers, strings, dates, enums, ...)  -> value.
//
// Nesting deeper than MaxDepth becomes a short summary, a reference back to an
// enclosing object becomes "[circular]", and a getter that throws shows its
// exception message instead of stopping the dump.

#nullable enable

using System;
using System.Collections;
using System.Collections.Generic;
using System.Linq;
using System.Numerics;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Tasks;

namespace TinkerPad;

/// <summary>Shows values in TinkerPad's output pane.</summary>
public static class Dump
{
    /// <summary>Levels of nesting shown; deeper structures are summarised.</summary>
    public const int MaxDepth = 5;

    /// <summary>Rows kept in a table, and items kept in a sequence or object.</summary>
    public const int MaxRows = 1000;

    private static readonly string Prefix = (char)0x1b + "]1337;TinkerPad=";
    private static readonly string Bel = ((char)0x07).ToString();

    private static readonly JsonSerializerOptions Json = new()
    {
        Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
    };

    /// <summary>
    /// Runs before the playground's own code: the dump escape sequence and any
    /// non-ASCII output must reach TinkerPad as UTF-8.
    /// </summary>
    [ModuleInitializer]
    internal static void UseUtf8Console()
    {
        Console.OutputEncoding = new UTF8Encoding(encoderShouldEmitUTF8Identifier: false);
    }

    /// <summary>Show <paramref name="value"/> in the output pane and return it.</summary>
    public static T Show<T>(T value, string? title = null)
    {
        var node = ToNode(value, title, typeof(T));
        Console.Out.Write(Prefix + node.ToJsonString(Json) + Bel);
        Console.Out.Flush();
        return value;
    }

    /// <summary>The dump node for <paramref name="value"/>, without writing it.</summary>
    /// <param name="declared">The static type, used to name a null value.</param>
    public static JsonObject ToNode(object? value, string? title = null, Type? declared = null)
    {
        var node = Describe(value, 0, new HashSet<object>(ReferenceEqualityComparer.Instance));
        if (value is null && declared is not null && declared != typeof(object)) node["type"] = TypeName(declared);
        if (title is not null) node["title"] = title;
        return node;
    }

    private static JsonObject Describe(object? value, int depth, HashSet<object> ancestors)
    {
        if (value is not null && !IsScalar(value) && TableRows(value) is { } rows)
            return Table(value, rows, depth, ancestors);
        if (value is null || IsScalar(value))
            return new JsonObject { ["v"] = 1, ["kind"] = "value", ["value"] = Scalar(value), ["type"] = ValueTypeName(value) };
        return new JsonObject { ["v"] = 1, ["kind"] = "tree", ["value"] = ToJson(value, depth, ancestors), ["type"] = TypeName(value.GetType()) };
    }

    /// <summary>The badge for a scalar: reflection objects by their public type.</summary>
    private static string ValueTypeName(object? value) => value switch
    {
        Type => "Type",
        MethodInfo => "MethodInfo",
        PropertyInfo => "PropertyInfo",
        FieldInfo => "FieldInfo",
        _ => TypeName(value?.GetType()),
    };

    // ---- tables ------------------------------------------------------------

    /// <summary>
    /// The first MaxRows + 1 items of a sequence whose items are all records,
    /// or null when it isn't one. Only this many items are ever enumerated, so
    /// an endless sequence is fine.
    /// </summary>
    private static List<object>? TableRows(object value)
    {
        if (value is string || value is IDictionary || IsGenericDictionary(value.GetType()) || value is not IEnumerable sequence)
            return null;
        var rows = new List<object>();
        try
        {
            foreach (var item in sequence)
            {
                if (item is null || !IsRecord(item)) return null;
                rows.Add(item);
                if (rows.Count > MaxRows) break;
            }
        }
        catch
        {
            return null;
        }
        return rows.Count == 0 ? null : rows;
    }

    private static JsonObject Table(object table, List<object> rows, int depth, HashSet<object> ancestors)
    {
        var truncated = rows.Count > MaxRows;
        if (truncated) rows.RemoveAt(rows.Count - 1);
        ancestors.Add(table);

        var columns = new List<string>();
        var declared = new Dictionary<string, HashSet<Type>>();
        var perRow = new List<List<(string Name, Type Declared, Func<object?> Read)>>();
        foreach (var row in rows)
        {
            var members = Members(row);
            perRow.Add(members);
            foreach (var (name, type, _) in members)
            {
                if (!declared.TryGetValue(name, out var types))
                {
                    columns.Add(name);
                    declared[name] = types = new HashSet<Type>();
                }
                types.Add(type);
            }
        }

        var body = new JsonArray();
        for (var r = 0; r < rows.Count; r++)
        {
            var row = rows[r];
            var cells = new JsonArray();
            if (ancestors.Contains(row))
            {
                foreach (var _ in columns) cells.Add("[circular]");
            }
            else
            {
                if (!row.GetType().IsValueType) ancestors.Add(row);
                var values = new Dictionary<string, Func<object?>>();
                foreach (var (name, _, read) in perRow[r]) values[name] = read;
                foreach (var column in columns)
                    cells.Add(values.TryGetValue(column, out var read) ? Cell(Read(read), depth + 2, ancestors) : null);
                ancestors.Remove(row);
            }
            body.Add(cells);
        }
        ancestors.Remove(table);

        var columnNodes = new JsonArray();
        foreach (var column in columns)
        {
            var types = declared[column];
            var node = new JsonObject { ["name"] = column };
            node["type"] = types.Count == 1 ? TypeName(types.First()) : "mixed";
            columnNodes.Add(node);
        }
        return new JsonObject
        {
            ["v"] = 1,
            ["kind"] = "table",
            ["columns"] = columnNodes,
            ["rows"] = body,
            ["truncated"] = truncated,
        };
    }

    /// <summary>A table cell: scalars inline, structures as nested nodes the app can open.</summary>
    private static JsonNode? Cell(object? value, int depth, HashSet<object> ancestors)
    {
        if (value is null || IsScalar(value)) return Scalar(value);
        if (ancestors.Contains(value)) return "[circular]";
        if (depth >= MaxDepth) return Summary(value);
        return Describe(value, depth, ancestors);
    }

    // ---- trees -------------------------------------------------------------

    private static JsonNode? ToJson(object? value, int depth, HashSet<object> ancestors)
    {
        if (value is null || IsScalar(value)) return Scalar(value);
        if (ancestors.Contains(value)) return "[circular]";
        if (depth >= MaxDepth) return Summary(value);
        var tracked = !value.GetType().IsValueType && ancestors.Add(value);
        try
        {
            switch (value)
            {
                case Exception e:
                {
                    var o = new JsonObject { ["type"] = TypeName(e.GetType()), ["message"] = e.Message };
                    if (e.StackTrace is { } stack) o["stackTrace"] = stack;
                    if (e.InnerException is { } inner) o["innerException"] = ToJson(inner, depth + 1, ancestors);
                    return o;
                }
                case IDictionary dictionary:
                    return Dictionary(Entries(dictionary), depth, ancestors);
                case IEnumerable sequence when IsGenericDictionary(value.GetType()):
                    return Dictionary(GenericEntries(sequence), depth, ancestors);
                case IEnumerable sequence:
                {
                    var array = new JsonArray();
                    var count = 0;
                    try
                    {
                        foreach (var item in sequence)
                        {
                            if (count++ == MaxRows)
                            {
                                array.Add("… more");
                                break;
                            }
                            array.Add(ToJson(item, depth + 1, ancestors));
                        }
                    }
                    catch (Exception e)
                    {
                        array.Add($"[error: {e.Message}]");
                    }
                    return array;
                }
                default:
                {
                    var o = new JsonObject();
                    var members = Members(value);
                    foreach (var (name, _, read) in members.Take(MaxRows))
                        o[name] = ToJson(Read(read), depth + 1, ancestors);
                    if (members.Count > MaxRows) o["…"] = $"… {members.Count - MaxRows} more";
                    return o;
                }
            }
        }
        finally
        {
            if (tracked) ancestors.Remove(value);
        }
    }

    private static JsonNode Dictionary(IEnumerable<(object Key, object? Value)> entries, int depth, HashSet<object> ancestors)
    {
        var list = entries.Take(MaxRows + 1).ToList();
        var more = list.Count > MaxRows;
        if (more) list.RemoveAt(list.Count - 1);
        if (list.All(e => IsScalar(e.Key)))
        {
            var o = new JsonObject();
            foreach (var (key, value) in list)
                o[ScalarText(key)] = ToJson(value, depth + 1, ancestors);
            if (more) o["…"] = "… more";
            return o;
        }
        var array = new JsonArray();
        foreach (var (key, value) in list)
            array.Add(new JsonObject { ["key"] = ToJson(key, depth + 1, ancestors), ["value"] = ToJson(value, depth + 1, ancestors) });
        if (more) array.Add("… more");
        return array;
    }

    private static IEnumerable<(object Key, object? Value)> Entries(IDictionary dictionary)
    {
        foreach (DictionaryEntry entry in dictionary) yield return (entry.Key, entry.Value);
    }

    private static IEnumerable<(object Key, object? Value)> GenericEntries(IEnumerable pairs)
    {
        foreach (var pair in pairs)
        {
            if (pair is null) continue;
            var type = pair.GetType();
            var key = type.GetProperty("Key")?.GetValue(pair);
            if (key is null) continue;
            yield return (key, type.GetProperty("Value")?.GetValue(pair));
        }
    }

    // ---- members -----------------------------------------------------------

    /// <summary>Public readable instance properties and fields, in declaration order.</summary>
    private static List<(string Name, Type Declared, Func<object?> Read)> Members(object value)
    {
        var type = value.GetType();
        var members = new List<(string, Type, Func<object?>)>();
        foreach (var property in type.GetProperties(BindingFlags.Public | BindingFlags.Instance))
        {
            if (!property.CanRead || property.GetIndexParameters().Length > 0) continue;
            members.Add((property.Name, property.PropertyType, () => property.GetValue(value)));
        }
        foreach (var field in type.GetFields(BindingFlags.Public | BindingFlags.Instance))
            members.Add((field.Name, field.FieldType, () => field.GetValue(value)));
        return members;
    }

    private static object? Read(Func<object?> read)
    {
        try
        {
            return read();
        }
        catch (TargetInvocationException e) when (e.InnerException is not null)
        {
            return $"[error: {e.InnerException.Message}]";
        }
        catch (Exception e)
        {
            return $"[error: {e.Message}]";
        }
    }

    /// <summary>An object a table row can be made of: it has members and isn't a collection.</summary>
    private static bool IsRecord(object value) =>
        !IsScalar(value)
        && value is not IEnumerable
        && value is not Exception
        && Members(value).Count > 0;

    private static bool IsGenericDictionary(Type type) =>
        type.GetInterfaces().Append(type).Any(i =>
            i.IsGenericType
            && (i.GetGenericTypeDefinition() == typeof(IDictionary<,>)
                || i.GetGenericTypeDefinition() == typeof(IReadOnlyDictionary<,>)));

    // ---- scalars -----------------------------------------------------------

    private static bool IsScalar(object? value) =>
        value is null
            or string or char or bool
            or sbyte or byte or short or ushort or int or uint or long or ulong
            or float or double or decimal or nint or nuint or BigInteger or Half
            or Enum or DateTime or DateTimeOffset or DateOnly or TimeOnly or TimeSpan
            or Guid or Uri or Version or Type or Delegate or MemberInfo
            or Task;

    /// <summary>A scalar as JSON. Numbers JavaScript can't hold exactly become strings.</summary>
    private static JsonNode? Scalar(object? value) => value switch
    {
        null => null,
        string s => s,
        char c => c.ToString(),
        bool b => b,
        sbyte or byte or short or ushort or int or uint => JsonValue.Create(Convert.ToInt64(value)),
        long l => Math.Abs(l) <= MaxSafeInteger ? JsonValue.Create(l) : l.ToString(),
        ulong u => u <= MaxSafeInteger ? JsonValue.Create(u) : u.ToString(),
        float f => float.IsFinite(f) ? JsonValue.Create(f) : f.ToString(System.Globalization.CultureInfo.InvariantCulture),
        double d => double.IsFinite(d) ? JsonValue.Create(d) : d.ToString(System.Globalization.CultureInfo.InvariantCulture),
        Half h => Half.IsFinite(h) ? JsonValue.Create((double)h) : h.ToString(System.Globalization.CultureInfo.InvariantCulture),
        decimal m => JsonValue.Create(m),
        _ => ScalarText(value),
    };

    private const long MaxSafeInteger = 9007199254740991;

    private static string ScalarText(object value) => value switch
    {
        DateTime dt => dt.ToString("o", System.Globalization.CultureInfo.InvariantCulture),
        DateTimeOffset dto => dto.ToString("o", System.Globalization.CultureInfo.InvariantCulture),
        DateOnly date => date.ToString("yyyy-MM-dd", System.Globalization.CultureInfo.InvariantCulture),
        TimeOnly time => time.ToString("HH:mm:ss.FFFFFFF", System.Globalization.CultureInfo.InvariantCulture),
        TimeSpan span => span.ToString("c", System.Globalization.CultureInfo.InvariantCulture),
        Type type => TypeName(type),
        Delegate del => $"[{TypeName(del.GetType())}]",
        MemberInfo member => $"{TypeName(member.DeclaringType)}.{member.Name}",
        Task task => $"[Task: {task.Status}]",
        IFormattable formattable => formattable.ToString(null, System.Globalization.CultureInfo.InvariantCulture),
        _ => value.ToString() ?? "",
    };

    private static string Summary(object value) => value switch
    {
        ICollection c => $"[{TypeName(value.GetType())}, {c.Count} items]",
        _ => $"[{TypeName(value.GetType())}]",
    };

    // ---- type names --------------------------------------------------------

    private static readonly Dictionary<Type, string> Aliases = new()
    {
        [typeof(bool)] = "bool", [typeof(byte)] = "byte", [typeof(sbyte)] = "sbyte", [typeof(char)] = "char",
        [typeof(short)] = "short", [typeof(ushort)] = "ushort", [typeof(int)] = "int", [typeof(uint)] = "uint",
        [typeof(long)] = "long", [typeof(ulong)] = "ulong", [typeof(float)] = "float", [typeof(double)] = "double",
        [typeof(decimal)] = "decimal", [typeof(string)] = "string", [typeof(object)] = "object",
        [typeof(nint)] = "nint", [typeof(nuint)] = "nuint",
    };

    /// <summary>A C#-style name: <c>int</c>, <c>List&lt;Person&gt;</c>, <c>int?</c>, <c>anonymous</c>.</summary>
    private static string TypeName(Type? type)
    {
        if (type is null) return "null";
        if (Aliases.TryGetValue(type, out var alias)) return alias;
        if (type.IsArray) return $"{TypeName(type.GetElementType())}[{new string(',', type.GetArrayRank() - 1)}]";
        if (Nullable.GetUnderlyingType(type) is { } underlying) return $"{TypeName(underlying)}?";
        if (type.Name.Contains("AnonymousType", StringComparison.Ordinal)) return "anonymous";
        if (!type.IsGenericType) return type.Name;
        var name = type.Name;
        var tick = name.IndexOf('`');
        if (tick >= 0) name = name[..tick];
        var args = string.Join(", ", type.GetGenericArguments().Select(TypeName));
        return name.StartsWith("ValueTuple", StringComparison.Ordinal) ? $"({args})" : $"{name}<{args}>";
    }
}

/// <summary>The <c>.Dump()</c> extension method.</summary>
public static class DumpExtensions
{
    /// <summary>Show <paramref name="value"/> in TinkerPad's output pane and return it.</summary>
    public static T Dump<T>(this T value, string? title = null) => TinkerPad.Dump.Show(value, title);
}
