// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/NurtCallException.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/NurtCallException.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;
using System.Globalization;

namespace Danaco.Nurt;

/// <summary>
/// Failure of a call: a status other than <see cref="CallStatus.Ok"/>, a protocol
/// error of <c>nurt serve</c>, an unexpected end of the <c>nurt</c> process, a missing
/// artifact or an incompatible interface version.
/// </summary>
/// <remarks>
/// Errors of the language are not exceptions; they are reported by
/// <see cref="NurtResponse.Error"/>, or by <see cref="NurtLanguageException"/> when
/// <see cref="RunOptions.ThrowOnLanguageError"/> is set.
/// </remarks>
public class NurtCallException : Exception
{
    /// <summary>Initializes an exception with the status <see cref="CallStatus.Internal"/>.</summary>
    public NurtCallException()
        : this(CallStatus.Internal, "call failed")
    {
    }

    /// <summary>Initializes an exception with the status <see cref="CallStatus.Internal"/> and a message.</summary>
    /// <param name="message">Description of the failure.</param>
    public NurtCallException(string message)
        : this(CallStatus.Internal, message)
    {
    }

    /// <summary>Initializes an exception with the status <see cref="CallStatus.Internal"/>, a message and a cause.</summary>
    /// <param name="message">Description of the failure.</param>
    /// <param name="innerException">Exception that caused the failure.</param>
    public NurtCallException(string message, Exception innerException)
        : this(CallStatus.Internal, message, innerException)
    {
    }

    /// <summary>Initializes an exception with a status and a message.</summary>
    /// <param name="status">Status of the failed call.</param>
    /// <param name="message">Description of the failure.</param>
    public NurtCallException(CallStatus status, string message)
        : base(message)
    {
        Status = status;
    }

    /// <summary>Initializes an exception with a status, a message and a cause.</summary>
    /// <param name="status">Status of the failed call.</param>
    /// <param name="message">Description of the failure.</param>
    /// <param name="innerException">Exception that caused the failure.</param>
    public NurtCallException(CallStatus status, string message, Exception? innerException)
        : base(message, innerException)
    {
        Status = status;
    }

    /// <summary>Status of the failed call.</summary>
    public CallStatus Status { get; }

    /// <summary>
    /// Name of <see cref="Status"/> as declared in <c>nurt.h</c>, for example
    /// <c>NURT_E_ARG</c>; the decimal value for a status unknown to this version.
    /// </summary>
    public string StatusName => GetStatusName(Status);

    /// <summary>
    /// Code of the JSON-RPC error returned by <c>nurt serve</c>, or <see langword="null"/>
    /// when the failure did not come from the protocol.
    /// </summary>
    public int? RpcErrorCode { get; init; }

    /// <summary>
    /// Details of the failure as JSON, from <c>nurt_last_error</c> or the <c>data</c>
    /// member of a JSON-RPC error; <see langword="null"/> when not available.
    /// </summary>
    public string? Detail { get; init; }

    /// <summary>Returns the name of a status as declared in <c>nurt.h</c>.</summary>
    /// <param name="status">Status value.</param>
    /// <returns>The header name, or the decimal value for an unknown status.</returns>
    public static string GetStatusName(CallStatus status) => status switch
    {
        CallStatus.Ok => "NURT_OK",
        CallStatus.Timeout => "NURT_E_TIMEOUT",
        CallStatus.Cancelled => "NURT_E_CANCELLED",
        CallStatus.InvalidArgument => "NURT_E_ARG",
        CallStatus.AbiMismatch => "NURT_E_ABI",
        CallStatus.Io => "NURT_E_IO",
        CallStatus.Internal => "NURT_E_INTERNAL",
        _ => ((int)status).ToString(CultureInfo.InvariantCulture),
    };
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
