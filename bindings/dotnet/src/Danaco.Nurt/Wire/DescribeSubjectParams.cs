// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System.Text.Json.Serialization;

namespace Danaco.Nurt;

/// <summary>Parameters of the <c>describe</c> method of <c>nurt serve</c>.</summary>
internal sealed class DescribeSubjectParams
{
    [JsonPropertyName("subject")]
    public string Subject { get; set; } = "core";

    [JsonPropertyName("language")]
    public string? Language { get; set; }
}
