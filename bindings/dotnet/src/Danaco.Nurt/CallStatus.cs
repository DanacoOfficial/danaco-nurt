// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

namespace Danaco.Nurt;

/// <summary>
/// Result of a call of the library, as defined by <c>nurt_status</c> in <c>nurt.h</c>.
/// </summary>
/// <remarks>
/// A status reports only failures of the call itself. Errors of the language are
/// carried by <see cref="NurtResponse.Error"/>. A later minor version of the interface
/// may add codes; an unknown non-zero value is a failure.
/// </remarks>
public enum CallStatus
{
    /// <summary>The call succeeded (<c>NURT_OK</c>).</summary>
    Ok = 0,

    /// <summary>The execution is still running after the timeout (<c>NURT_E_TIMEOUT</c>).</summary>
    Timeout = 1,

    /// <summary>The execution ended because it was cancelled (<c>NURT_E_CANCELLED</c>).</summary>
    Cancelled = 2,

    /// <summary>Invalid argument: null value, invalid UTF-8 or invalid JSON (<c>NURT_E_ARG</c>).</summary>
    InvalidArgument = 3,

    /// <summary>Incompatible interface version (<c>NURT_E_ABI</c>).</summary>
    AbiMismatch = 4,

    /// <summary>A file, the journal, the state directory or an artifact cannot be opened or read (<c>NURT_E_IO</c>).</summary>
    Io = 5,

    /// <summary>Internal failure, including an unexpected end of the <c>nurt</c> process (<c>NURT_E_INTERNAL</c>).</summary>
    Internal = 6,
}
