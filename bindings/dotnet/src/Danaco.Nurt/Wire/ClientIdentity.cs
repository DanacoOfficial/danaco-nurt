using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class ClientIdentity
{
    [JsonPropertyName("name")]
    public string Name { get; set; } = WireFormat.ClientName;

    [JsonPropertyName("version")]
    public string Version { get; set; } = WireFormat.ClientVersion;
}
