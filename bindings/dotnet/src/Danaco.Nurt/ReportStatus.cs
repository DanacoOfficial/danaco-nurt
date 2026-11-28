// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/ReportStatus.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/ReportStatus.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

namespace Danaco.Nurt;

/// <summary>Outcome of a program, the field <c>status</c> of the report <c>nurt-report/1</c>.</summary>
public enum ReportStatus
{
    /// <summary>A value introduced by a later version of the report format.</summary>
    Unknown = 0,

    /// <summary>The program completed (<c>ok</c>).</summary>
    Ok = 1,

    /// <summary>The effect was computed without changes (<c>planned</c>).</summary>
    Planned = 2,

    /// <summary>The program continues in the background (<c>running</c>).</summary>
    Running = 3,

    /// <summary>Part of the changes was applied (<c>partial</c>).</summary>
    Partial = 4,

    /// <summary>The changes were rolled back (<c>rolled_back</c>).</summary>
    RolledBack = 5,

    /// <summary>The program failed (<c>failed</c>).</summary>
    Failed = 6,
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
