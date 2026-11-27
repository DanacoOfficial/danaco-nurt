// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Rpc/RpcErrors.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Rpc/RpcErrors.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System.Globalization;
using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>
/// Maps JSON-RPC errors of <c>nurt serve</c> to call statuses. Codes from -32000 to -32099
/// carry a status of <c>nurt.h</c> as <c>-32000 - status</c>; the codes reserved by
/// JSON-RPC 2.0 are failures of the request itself.
/// </summary>
internal static class RpcErrors
{
    public const int ParseError = -32700;
    public const int InvalidRequest = -32600;
    public const int MethodNotFound = -32601;
    public const int InvalidParams = -32602;
    public const int InternalError = -32603;
    public const int StatusBase = -32000;
    public const int StatusLast = -32099;

    public static CallStatus ToStatus(int code) => code switch
    {
        ParseError or InvalidRequest or InvalidParams => CallStatus.InvalidArgument,
        MethodNotFound => CallStatus.AbiMismatch,
        <= StatusBase and >= StatusLast => (CallStatus)(StatusBase - code),
        _ => CallStatus.Internal,
    };

    private static CallStatus? StatusByName(string? name) => name switch
    {
        "NURT_E_TIMEOUT" => CallStatus.Timeout,
        "NURT_E_CANCELLED" => CallStatus.Cancelled,
        "NURT_E_ARG" => CallStatus.InvalidArgument,
        "NURT_E_ABI" => CallStatus.AbiMismatch,
        "NURT_E_IO" => CallStatus.Io,
        "NURT_E_INTERNAL" => CallStatus.Internal,
        _ => null,
    };

    /// <summary>
    /// Identifier of the request an event belongs to: the member <c>request</c>, which is the
    /// identifier as text (a number is accepted as well), or <c>request_id</c> as a number.
    /// </summary>
    public static long? RequestIdOf(JsonElement parameters)
    {
        JsonElement? request = JsonRead.Property(parameters, "request");
        if (request is { ValueKind: JsonValueKind.String } text)
        {
            return long.TryParse(text.GetString(), NumberStyles.None, CultureInfo.InvariantCulture, out long parsed) ? parsed : null;
        }

        return JsonRead.Int64(parameters, "request") ?? JsonRead.Int64(parameters, "request_id");
    }

    /// <summary>
    /// Response of a cancelled <c>run</c>: the server answers it with the error
    /// <c>NURT_E_CANCELLED</c> whose <c>data.response</c> is the response of the cancelled execution.
    /// </summary>
    public static JsonElement? CancelledResponse(JsonElement error)
    {
        if (JsonRead.Int32(error, "code") != StatusBase - (int)CallStatus.Cancelled
            || JsonRead.Property(error, "data") is not { ValueKind: JsonValueKind.Object } data
            || JsonRead.Property(data, "response") is not { ValueKind: JsonValueKind.Object } response)
        {
            return null;
        }

        return response.Clone();
    }

    public static NurtCallException ToException(JsonElement error, string method)
    {
        int code = JsonRead.Int32(error, "code") ?? InternalError;
        string message = JsonRead.String(error, "message") ?? "request failed";
        JsonElement? data = JsonRead.Property(error, "data");
        CallStatus status = JsonRead.Property(error, "data") is { } detail && StatusByName(JsonRead.String(detail, "status")) is { } named
            ? named
            : ToStatus(code);
        return new NurtCallException(status, method + ": " + message)
        {
            RpcErrorCode = code,
            Detail = data?.GetRawText(),
        };
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
