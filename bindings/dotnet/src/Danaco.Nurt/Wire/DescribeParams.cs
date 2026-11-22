using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class DescribeParams
{
    [JsonPropertyName("layer")]
    public string Layer { get; set; } = "core";

    [JsonPropertyName("name")]
    public string? Name { get; set; }

    [JsonPropertyName("language")]
    public string? Language { get; set; }
}
