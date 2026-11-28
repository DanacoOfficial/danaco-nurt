// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Verbosity.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Verbosity.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

namespace Danaco.Nurt;

/// <summary>Amount of detail of the rendered result.</summary>
public enum Verbosity
{
    /// <summary>Only the outcome (<c>quiet</c>).</summary>
    Quiet = 0,

    /// <summary>Outcome and a sample of changes (<c>normal</c>).</summary>
    Normal = 1,

    /// <summary>All details (<c>detailed</c>).</summary>
    Detailed = 2,
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
