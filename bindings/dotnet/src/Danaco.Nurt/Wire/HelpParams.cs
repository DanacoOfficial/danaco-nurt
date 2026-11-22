using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class HelpParams
{
    [JsonPropertyName("topic")]
    public string? Topic { get; set; }
}
