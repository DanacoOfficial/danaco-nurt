// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/OutputFormat.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/OutputFormat.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

namespace Danaco.Nurt;

/// <summary>Format of the rendered result.</summary>
public enum OutputFormat
{
    /// <summary>Plain text (<c>text</c>).</summary>
    Text = 0,

    /// <summary>The report only, without rendered text (<c>json</c>).</summary>
    Json = 1,

    /// <summary>Markdown (<c>markdown</c>).</summary>
    Markdown = 2,

    /// <summary>Short text for constrained contexts (<c>compact</c>).</summary>
    Compact = 3,
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
