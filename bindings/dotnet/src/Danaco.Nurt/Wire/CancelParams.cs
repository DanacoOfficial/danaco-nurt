using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class CancelParams
{
    [JsonPropertyName("request_id")]
    public long RequestId { get; set; }
}
