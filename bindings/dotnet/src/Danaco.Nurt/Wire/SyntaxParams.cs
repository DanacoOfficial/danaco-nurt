using System.Text.Json;
using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class SyntaxParams
{
    [JsonPropertyName("program")]
    public string? Program { get; set; }

    [JsonPropertyName("ast")]
    public JsonElement? Ast { get; set; }

    [JsonPropertyName("variant")]
    public string Variant { get; set; } = "a";
}
