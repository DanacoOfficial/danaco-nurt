using System.Text.Json.Serialization;

namespace Danaco.Nurt;

[JsonSourceGenerationOptions(DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull)]
[JsonSerializable(typeof(RequestEnvelope))]
[JsonSerializable(typeof(SyntaxParams))]
[JsonSerializable(typeof(PageParams))]
[JsonSerializable(typeof(DescribeParams))]
[JsonSerializable(typeof(HelpParams))]
[JsonSerializable(typeof(SchemaParams))]
[JsonSerializable(typeof(SpaceParams))]
[JsonSerializable(typeof(CancelParams))]
[JsonSerializable(typeof(InitializeParams))]
internal sealed partial class WireJsonContext : JsonSerializerContext
{
}
