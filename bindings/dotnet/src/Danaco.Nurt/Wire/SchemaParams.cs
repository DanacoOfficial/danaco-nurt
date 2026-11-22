using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class SchemaParams
{
    [JsonPropertyName("which")]
    public string Which { get; set; } = string.Empty;
}
