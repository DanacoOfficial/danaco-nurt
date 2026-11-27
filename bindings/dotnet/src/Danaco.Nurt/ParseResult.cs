// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/ParseResult.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/ParseResult.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
