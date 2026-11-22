using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class PageParams
{
    [JsonPropertyName("cursor")]
    public string Cursor { get; set; } = string.Empty;

    [JsonPropertyName("limit")]
    public int? Limit { get; set; }
}
