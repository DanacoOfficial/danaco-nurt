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
    public AbiVersion Abi { get; set; } = new();

    [JsonPropertyName("language")]
    public string? Language { get; set; }

    [JsonPropertyName("session")]
    public string? Session { get; set; }

    [JsonPropertyName("config_overrides")]
    public JsonElement? ConfigOverrides { get; set; }

    [JsonPropertyName("capabilities")]
    public ClientCapabilities Capabilities { get; set; } = new();
}
