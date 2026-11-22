using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class ClientCapabilities
{
    [JsonPropertyName("events")]
    public bool Events { get; set; } = true;
}
