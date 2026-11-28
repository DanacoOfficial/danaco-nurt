// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/AmbiguityCandidate.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/AmbiguityCandidate.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>
/// One reading of an ambiguous program, listed by an <c>E_AMBIGUOUS</c> error. A candidate
/// is run with <see cref="Request.FromAccept(string)"/>.
/// </summary>
public sealed class AmbiguityCandidate
{
    internal AmbiguityCandidate(JsonElement element)
    {
        Rank = JsonRead.Int32(element, "rank") ?? 0;
        Ref = JsonRead.String(element, "ref");
        Program = JsonRead.String(element, "program") ?? string.Empty;
        Why = JsonRead.String(element, "why");
    }

    /// <summary>Rank of the candidate, from 1 for the most likely reading.</summary>
    public int Rank { get; }

    /// <summary>Identifier passed to <see cref="Request.FromAccept(string)"/>; <see langword="null"/> when not reported.</summary>
    public string? Ref { get; }

    /// <summary>Canonical text of the program read this way.</summary>
    public string Program { get; }

    /// <summary>Reason for the reading; <see langword="null"/> when not reported.</summary>
    public string? Why { get; }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
