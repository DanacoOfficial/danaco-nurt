using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>
/// Subject of a request envelope <c>nurt-request/1</c>: a program text, a syntax tree,
/// a cursor of a further page, or a candidate accepted after an ambiguity error.
/// </summary>
/// <remarks>Exactly one of the subjects is set; instances are created by the factory methods.</remarks>
public sealed class Request
{
    private Request(string? program, NotationVariant? variant, JsonElement? ast, string? cursor, string? accept)
    {
        Program = program;
        Variant = variant;
        Ast = ast;
        Cursor = cursor;
        Accept = accept;
    }

    /// <summary>Program text; <see langword="null"/> for the other subjects.</summary>
    public string? Program { get; }

    /// <summary>Notation variant of <see cref="Program"/>; <see langword="null"/> to detect it.</summary>
    public NotationVariant? Variant { get; }

    /// <summary>Syntax tree <c>nurt-ast/1</c>; <see langword="null"/> for the other subjects.</summary>
    public JsonElement? Ast { get; }

    /// <summary>Cursor of a further page; <see langword="null"/> for the other subjects.</summary>
    public string? Cursor { get; }

    /// <summary>Identifier of an accepted candidate; <see langword="null"/> for the other subjects.</summary>
    public string? Accept { get; }

    /// <summary>Creates a request that runs a program text.</summary>
    /// <param name="program">Program text in any notation variant.</param>
    /// <param name="variant">Notation variant; <see langword="null"/> to detect it.</param>
    /// <returns>The request.</returns>
    /// <exception cref="System.ArgumentNullException"><paramref name="program"/> is <see langword="null"/>.</exception>
    public static Request FromProgram(string program, NotationVariant? variant = null)
    {
        Guard.NotNull(program, nameof(program));
        return new Request(program, variant, null, null, null);
    }

    /// <summary>Creates a request that runs a syntax tree.</summary>
    /// <param name="ast">Syntax tree <c>nurt-ast/1</c> as a JSON object.</param>
    /// <returns>The request.</returns>
    public static Request FromAst(JsonElement ast) => new(null, null, ast.Clone(), null, null);

    /// <summary>Creates a request for a further page of an earlier result.</summary>
    /// <param name="cursor">Cursor from <see cref="Response.NextCursor"/>.</param>
    /// <returns>The request.</returns>
    /// <exception cref="System.ArgumentNullException"><paramref name="cursor"/> is <see langword="null"/>.</exception>
    public static Request FromCursor(string cursor)
    {
        Guard.NotNull(cursor, nameof(cursor));
        return new Request(null, null, null, cursor, null);
    }

    /// <summary>Creates a request that runs a candidate listed by an <c>E_AMBIGUOUS</c> error.</summary>
    /// <param name="candidate">Identifier of the candidate.</param>
    /// <returns>The request.</returns>
    /// <exception cref="System.ArgumentNullException"><paramref name="candidate"/> is <see langword="null"/>.</exception>
    public static Request FromAccept(string candidate)
    {
        Guard.NotNull(candidate, nameof(candidate));
        return new Request(null, null, null, null, candidate);
    }
}
