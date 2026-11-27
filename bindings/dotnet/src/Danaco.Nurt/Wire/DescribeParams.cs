// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Wire/DescribeParams.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Wire/DescribeParams.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class DescribeParams
{
    [JsonPropertyName("layer")]
    public string Layer { get; set; } = "core";

    [JsonPropertyName("name")]
    public string? Name { get; set; }

    [JsonPropertyName("language")]
    public string? Language { get; set; }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
