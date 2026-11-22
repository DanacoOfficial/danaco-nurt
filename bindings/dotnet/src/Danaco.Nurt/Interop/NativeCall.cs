// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>Turns a status of the library into <see cref="NurtCallException"/>.</summary>
internal static unsafe class NativeCall
{
    /// <summary>
    /// Function with one zero-terminated input and one output buffer; the engine, when
    /// needed, is captured by the caller.
    /// </summary>
    public delegate CallStatus OneOutput(byte* input, NativeBuffer* output);

    public static void Check(CallStatus status, string function)
    {
        if (status != CallStatus.Ok)
        {
            throw Failure(status, function);
        }
    }

    /// <summary>
    /// Builds the exception for a failed call. Must run on the thread of the failed call,
    /// because <c>nurt_last_error</c> keeps the record per thread.
    /// </summary>
    public static NurtCallException Failure(CallStatus status, string function)
    {
        string? detail = LastError();
        string? message = detail is null ? null : MessageOf(detail);
        return new NurtCallException(status, function + ": " + (message ?? NurtCallException.GetStatusName(status)))
        {
            Detail = detail,
        };
    }

    /// <summary>Calls a function with one output buffer and returns the buffer as text; the buffer is always released.</summary>
    public static string Text(byte[]? input, string function, OneOutput call)
    {
        NativeBuffer output = default;
        try
        {
            fixed (byte* pointer = input)
            {
                Check(call(pointer, &output), function);
            }

            return output.ReadString();
        }
        finally
        {
            NativeMethods.BufferFree(&output);
        }
    }

    /// <summary>Calls a function with one output buffer and parses the buffer as JSON.</summary>
    public static JsonElement Json(byte[]? input, string function, OneOutput call) =>
        JsonRead.Parse(Text(input, function, call), function + " result");

    private static string? LastError()
    {
        NativeBuffer buffer = default;
        try
        {
            return NativeMethods.LastError(IntPtr.Zero, &buffer) == CallStatus.Ok && !buffer.IsEmpty
                ? buffer.ReadString()
                : null;
        }
        finally
        {
            NativeMethods.BufferFree(&buffer);
        }
    }

    private static string? MessageOf(string detail)
    {
        try
        {
            using JsonDocument document = JsonDocument.Parse(detail);
            return JsonRead.String(document.RootElement, "message");
        }
        catch (JsonException)
        {
            return null;
        }
    }
}
