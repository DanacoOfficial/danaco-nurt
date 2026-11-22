using System.Text.Json.Serialization;

namespace Danaco.Nurt;

internal sealed class AbiVersion
{
    [JsonPropertyName("major")]
    public int Major { get; set; } = NativeMethods.AbiMajor;

    [JsonPropertyName("minor")]
    public int Minor { get; set; } = NativeMethods.AbiMinor;
}
