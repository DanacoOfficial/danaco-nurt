using System.Collections.Generic;
using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class EnvelopeOptions
{
    [JsonPropertyName("preview")]
    public bool? Preview { get; set; }

    [JsonPropertyName("base")]
    public string? Base { get; set; }

    [JsonPropertyName("limit")]
    public int? Limit { get; set; }

    [JsonPropertyName("fields")]
    public IReadOnlyList<string>? Fields { get; set; }

    [JsonPropertyName("format")]
    public string? Format { get; set; }

    [JsonPropertyName("verbosity")]
    public string? Verbosity { get; set; }

    [JsonPropertyName("diff")]
    public string? Diff { get; set; }

    [JsonPropertyName("diff_context")]
    public int? DiffContext { get; set; }

    [JsonPropertyName("max_chars")]
    public int? MaxChars { get; set; }

    [JsonPropertyName("save_to")]
    public string? SaveTo { get; set; }

    [JsonPropertyName("timeout")]
    public string? Timeout { get; set; }

    [JsonPropertyName("session")]
    public string? Session { get; set; }

    [JsonPropertyName("language")]
    public string? Language { get; set; }

    [JsonPropertyName("script")]
    public bool? Script { get; set; }
}
