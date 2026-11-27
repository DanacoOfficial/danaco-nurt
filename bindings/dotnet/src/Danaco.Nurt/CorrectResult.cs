// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/CorrectResult.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/CorrectResult.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>Result of correcting the written form of a program without running it.</summary>
public sealed class CorrectResult
{
    internal CorrectResult(JsonElement result)
    {
        Result = result;
        Program = JsonRead.String(result, "program");
    }

    /// <summary>Corrected program text; <see langword="null"/> when the program cannot be corrected.</summary>
    public string? Program { get; }

    /// <summary>The whole result as JSON, including the list of corrections.</summary>
    public JsonElement Result { get; }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
