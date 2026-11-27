// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Wire/RequestEnvelope.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Wire/RequestEnvelope.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System.Collections.Generic;
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

    [JsonPropertyName("params")]
    public Dictionary<string, JsonElement>? Params { get; set; }

    [JsonPropertyName("options")]
    public EnvelopeOptions? Options { get; set; }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
