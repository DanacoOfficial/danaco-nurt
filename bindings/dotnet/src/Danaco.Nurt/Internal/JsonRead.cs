// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.Collections.Generic;
using System.Text.Json;

namespace Danaco.Nurt;

internal static class JsonRead
{
    public static JsonElement Parse(string json, string what)
    {
        try
        {
            using JsonDocument document = JsonDocument.Parse(json);
            return document.RootElement.Clone();
        }
        catch (JsonException exception)
        {
            throw new NurtCallException(CallStatus.Internal, what + " is not valid JSON: " + exception.Message, exception);
        }
    }

    public static JsonElement? Property(JsonElement element, string name)
    {
        if (element.ValueKind == JsonValueKind.Object
            && element.TryGetProperty(name, out JsonElement value)
            && value.ValueKind != JsonValueKind.Null)
        {
            return value;
        }

        return null;
    }

    public static string? String(JsonElement element, string name)
    {
        JsonElement? value = Property(element, name);
        return value is { ValueKind: JsonValueKind.String } text ? text.GetString() : null;
    }

    public static long? Int64(JsonElement element, string name)
    {
        JsonElement? value = Property(element, name);
        return value is { ValueKind: JsonValueKind.Number } number && number.TryGetInt64(out long result) ? result : null;
    }

    public static int? Int32(JsonElement element, string name)
    {
        JsonElement? value = Property(element, name);
        return value is { ValueKind: JsonValueKind.Number } number && number.TryGetInt32(out int result) ? result : null;
    }

    public static IReadOnlyList<string> Strings(JsonElement element, string name)
    {
        JsonElement? value = Property(element, name);
        if (value is not { ValueKind: JsonValueKind.Array } array)
        {
            return Array.Empty<string>();
        }

        var result = new List<string>(array.GetArrayLength());
        foreach (JsonElement item in array.EnumerateArray())
        {
            if (item.ValueKind == JsonValueKind.String)
            {
                result.Add(item.GetString()!);
            }
        }

        return result;
    }
}
