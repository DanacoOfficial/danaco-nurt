// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/NurtLanguageException.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/NurtLanguageException.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;

namespace Danaco.Nurt;

/// <summary>
/// Error of the language raised in place of a response when
/// <see cref="RunOptions.ThrowOnLanguageError"/> is set.
/// </summary>
public class NurtLanguageException : Exception
{
    /// <summary>Initializes an exception without a response.</summary>
    public NurtLanguageException()
        : base("language error")
    {
    }

    /// <summary>Initializes an exception with a message and without a response.</summary>
    /// <param name="message">Description of the error.</param>
    public NurtLanguageException(string message)
        : base(message)
    {
    }

    /// <summary>Initializes an exception with a message and a cause, without a response.</summary>
    /// <param name="message">Description of the error.</param>
    /// <param name="innerException">Exception that caused the error.</param>
    public NurtLanguageException(string message, Exception innerException)
        : base(message, innerException)
    {
    }

    /// <summary>Initializes an exception from a response that carries an error.</summary>
    /// <param name="response">Response whose <see cref="NurtResponse.Error"/> is set.</param>
    public NurtLanguageException(NurtResponse response)
        : base(FormatMessage(response))
    {
        Response = response;
    }

    /// <summary>Response that carries the error; <see langword="null"/> for the other constructors.</summary>
    public NurtResponse? Response { get; }

    /// <summary>Error of the language; <see langword="null"/> for the other constructors.</summary>
    public LanguageError? Error => Response?.Error;

    private static string FormatMessage(NurtResponse response)
    {
        Guard.NotNull(response, nameof(response));
        LanguageError? error = response.Error;
        return error is null ? "language error" : error.Code + ": " + error.Message;
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
