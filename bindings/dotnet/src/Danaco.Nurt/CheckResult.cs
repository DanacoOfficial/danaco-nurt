// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/CheckResult.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/CheckResult.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>
/// Result of binding and classifying a request without running it: bound tree, targets,
/// paths, external programs, network addresses, corrections and notable effects.
/// </summary>
public sealed class CheckResult
{
    internal CheckResult(JsonElement result, JsonElement? diagnostics)
    {
        Result = result;
        Diagnostics = diagnostics;
        Error = LanguageError.FromMember(result) ?? LanguageError.FromMember(diagnostics);
    }

    /// <summary>Classification of the request as JSON, in the format <c>nurt-check/1</c>.</summary>
    public JsonElement Result { get; }

    /// <summary>Diagnostics of binding as JSON; <see langword="null"/> when there are none.</summary>
    public JsonElement? Diagnostics { get; }

    /// <summary>Error of the language raised while binding the request; <see langword="null"/> when the request was bound.</summary>
    public LanguageError? Error { get; }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
