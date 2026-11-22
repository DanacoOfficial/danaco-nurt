// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System.Text.Json;
using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class InitializeParams
{
    [JsonPropertyName("protocol")]
    public string Protocol { get; set; } = WireFormat.Protocol;

    [JsonPropertyName("client")]
    public ClientIdentity Client { get; set; } = new();

    [JsonPropertyName("abi")]
    public int Abi { get; set; } = (int)NativeMethods.AbiHeaderVersion;

    [JsonPropertyName("language")]
    public string? Language { get; set; }

    [JsonPropertyName("session")]
    public string? Session { get; set; }

    [JsonPropertyName("config_overrides")]
    public JsonElement? ConfigOverrides { get; set; }

    [JsonPropertyName("capabilities")]
    public ClientCapabilities Capabilities { get; set; } = new();
}
