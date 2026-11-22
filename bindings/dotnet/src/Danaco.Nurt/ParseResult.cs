using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>Result of parsing a program text into a syntax tree.</summary>
public sealed class ParseResult
{
    internal ParseResult(JsonElement? ast, JsonElement? diagnostics, JsonElement? error)
    {
        Ast = ast;
        Diagnostics = diagnostics;
        Error = error is { ValueKind: JsonValueKind.Object } value ? new LanguageError(value) : LanguageError.FromMember(diagnostics);
    }

    /// <summary>Syntax tree <c>nurt-ast/1</c>; <see langword="null"/> when the text could not be parsed.</summary>
    public JsonElement? Ast { get; }

    /// <summary>Diagnostics with positions and messages; <see langword="null"/> when there are none.</summary>
    public JsonElement? Diagnostics { get; }

    /// <summary>Error of the language that prevented parsing, for example <c>E_LEXICAL</c>; <see langword="null"/> when the text was parsed.</summary>
    public LanguageError? Error { get; }
}
