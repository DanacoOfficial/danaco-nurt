// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class ClientIdentity
{
    [JsonPropertyName("name")]
    public string Name { get; set; } = WireFormat.ClientName;

    [JsonPropertyName("version")]
    public string Version { get; set; } = WireFormat.ClientVersion;
}
