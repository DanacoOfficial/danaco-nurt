using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class SpaceParams
{
    [JsonPropertyName("volume")]
    public string? Volume { get; set; }
}
