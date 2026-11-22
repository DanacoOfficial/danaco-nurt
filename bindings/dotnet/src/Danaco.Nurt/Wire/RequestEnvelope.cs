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
