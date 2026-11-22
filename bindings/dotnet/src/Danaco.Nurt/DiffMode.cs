// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

namespace Danaco.Nurt;

/// <summary>Form of the differences in the report.</summary>
public enum DiffMode
{
    /// <summary>No differences (<c>none</c>).</summary>
    None = 0,

    /// <summary>Counts of changed lines (<c>stat</c>).</summary>
    Stat = 1,

    /// <summary>Unified differences (<c>unified</c>).</summary>
    Unified = 2,
}
