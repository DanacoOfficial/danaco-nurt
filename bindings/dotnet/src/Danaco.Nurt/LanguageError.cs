// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/LanguageError.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/LanguageError.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System.Collections.Generic;
using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>
/// Error of the language carried by a report: a stable <c>E_*</c> code, the stage and
/// position where it was raised, and the ways to fix it.
/// </summary>
public sealed class LanguageError
{
    internal LanguageError(JsonElement element)
    {
        Element = element;
        Code = JsonRead.String(element, "code") ?? string.Empty;
        Message = JsonRead.String(element, "message") ?? string.Empty;
        JsonElement? at = JsonRead.Property(element, "at");
        if (at is { ValueKind: JsonValueKind.Object } position)
        {
            Stage = JsonRead.String(position, "stage");
            Line = JsonRead.Int32(position, "line");
            Column = JsonRead.Int32(position, "column");
            Length = JsonRead.Int32(position, "length");
        }

        Fix = JsonRead.Strings(element, "fix");
        Help = JsonRead.String(element, "help");
        Candidates = ReadCandidates(element);
    }

    /// <summary>Stable error code, for example <c>E_SYNTAX</c>.</summary>
    public string Code { get; }

    /// <summary>Description of the error.</summary>
    public string Message { get; }

    /// <summary>Execution stage in which the error was raised (<c>at.stage</c>), for example <c>1</c> or <c>5a</c>; <see langword="null"/> when not reported.</summary>
    public string? Stage { get; }

    /// <summary>Line of the program text, from 1; <see langword="null"/> when not reported.</summary>
    public int? Line { get; }

    /// <summary>Column of the program text, from 1; <see langword="null"/> when not reported.</summary>
    public int? Column { get; }

    /// <summary>Length of the reported fragment; <see langword="null"/> when not reported.</summary>
    public int? Length { get; }

    /// <summary>Corrected forms of the program or steps that remove the error.</summary>
    public IReadOnlyList<string> Fix { get; }

    /// <summary>Help topic that describes the error; <see langword="null"/> when not reported.</summary>
    public string? Help { get; }

    /// <summary>Candidates of an <c>E_AMBIGUOUS</c> error, best first.</summary>
    public IReadOnlyList<AmbiguityCandidate> Candidates { get; }

    /// <summary>The whole error object as JSON.</summary>
    public JsonElement Element { get; }

    /// <summary>Reads the member <c>error</c> of a result object; <see langword="null"/> when the member is absent.</summary>
    internal static LanguageError? FromMember(JsonElement? container) =>
        container is { ValueKind: JsonValueKind.Object } value && JsonRead.Property(value, "error") is { ValueKind: JsonValueKind.Object } error
            ? new LanguageError(error)
            : null;

    private static List<AmbiguityCandidate> ReadCandidates(JsonElement element)
    {
        var result = new List<AmbiguityCandidate>();
        JsonElement? candidates = JsonRead.Property(element, "candidates");
        if (candidates is { ValueKind: JsonValueKind.Array } array)
        {
            foreach (JsonElement item in array.EnumerateArray())
            {
                if (item.ValueKind == JsonValueKind.Object)
                {
                    result.Add(new AmbiguityCandidate(item));
                }
            }
        }

        return result;
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
