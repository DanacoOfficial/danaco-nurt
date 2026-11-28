// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Request.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Request.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System.Collections.Generic;
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

    /// <summary>
    /// Values of the parameters of a script that starts with <c>params { ... }</c>, by name;
    /// <see langword="null"/> when the request has none.
    /// </summary>
    public IReadOnlyDictionary<string, JsonElement>? Parameters { get; private init; }

    /// <summary>Returns a copy of this request that carries the values of the parameters of the script.</summary>
    /// <param name="parameters">Values by name: typed JSON, or text that the declared type reads.</param>
    /// <returns>The request with the parameters.</returns>
    /// <exception cref="System.ArgumentNullException"><paramref name="parameters"/> is <see langword="null"/>.</exception>
    public Request WithParameters(IReadOnlyDictionary<string, JsonElement> parameters)
    {
        Guard.NotNull(parameters, nameof(parameters));
        var copy = new Dictionary<string, JsonElement>();
        foreach (KeyValuePair<string, JsonElement> parameter in parameters)
        {
            copy[parameter.Key] = parameter.Value.Clone();
        }

        return new Request(Program, Variant, Ast, Cursor, Accept) { Parameters = copy };
    }

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
    /// <param name="cursor">Cursor from <see cref="NurtResponse.NextCursor"/>.</param>
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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
