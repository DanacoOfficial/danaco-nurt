// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Wire/WireJsonContext.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Wire/WireJsonContext.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System.Text.Json.Serialization;

namespace Danaco.Nurt;

[JsonSourceGenerationOptions(DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull)]
[JsonSerializable(typeof(RequestEnvelope))]
[JsonSerializable(typeof(SyntaxParams))]
[JsonSerializable(typeof(PageParams))]
[JsonSerializable(typeof(DescribeParams))]
[JsonSerializable(typeof(DescribeSubjectParams))]
[JsonSerializable(typeof(HelpParams))]
[JsonSerializable(typeof(SchemaParams))]
[JsonSerializable(typeof(SpaceParams))]
[JsonSerializable(typeof(CancelParams))]
[JsonSerializable(typeof(InitializeParams))]
internal sealed partial class WireJsonContext : JsonSerializerContext
{
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
