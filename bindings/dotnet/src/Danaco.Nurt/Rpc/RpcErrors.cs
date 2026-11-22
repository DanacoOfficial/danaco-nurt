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

    public static CallException ToException(JsonElement error, string method)
    {
        int code = JsonRead.Int32(error, "code") ?? InternalError;
        string message = JsonRead.String(error, "message") ?? "request failed";
        JsonElement? data = JsonRead.Property(error, "data");
        return new CallException(ToStatus(code), method + ": " + message)
        {
            RpcErrorCode = code,
            Detail = data?.GetRawText(),
        };
    }
}
