// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System.Text.Json;
using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class RequestEnvelope
{
    [JsonPropertyName("nurt")]
    public string Nurt { get; set; } = WireFormat.EnvelopeVersion;

    [JsonPropertyName("program")]
    public string? Program { get; set; }

    [JsonPropertyName("variant")]
    public string? Variant { get; set; }

    [JsonPropertyName("ast")]
    public JsonElement? Ast { get; set; }

    [JsonPropertyName("cursor")]
    public string? Cursor { get; set; }

    [JsonPropertyName("accept")]
    public string? Accept { get; set; }

    [JsonPropertyName("options")]
    public EnvelopeOptions? Options { get; set; }
}
