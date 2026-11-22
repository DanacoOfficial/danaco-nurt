using System;

namespace Danaco.Nurt;

/// <summary>
/// Error of the language raised in place of a response when
/// <see cref="RunOptions.ThrowOnLanguageError"/> is set.
/// </summary>
public class LanguageException : Exception
{
    /// <summary>Initializes an exception without a response.</summary>
    public LanguageException()
        : base("language error")
    {
    }

    /// <summary>Initializes an exception with a message and without a response.</summary>
    /// <param name="message">Description of the error.</param>
    public LanguageException(string message)
        : base(message)
    {
    }

    /// <summary>Initializes an exception with a message and a cause, without a response.</summary>
    /// <param name="message">Description of the error.</param>
    /// <param name="innerException">Exception that caused the error.</param>
    public LanguageException(string message, Exception innerException)
        : base(message, innerException)
    {
    }

    /// <summary>Initializes an exception from a response that carries an error.</summary>
    /// <param name="response">Response whose <see cref="Nurt.Response.Error"/> is set.</param>
    public LanguageException(Response response)
        : base(FormatMessage(response))
    {
        Response = response;
    }

    /// <summary>Response that carries the error; <see langword="null"/> for the other constructors.</summary>
    public Response? Response { get; }

    /// <summary>Error of the language; <see langword="null"/> for the other constructors.</summary>
    public LanguageError? Error => Response?.Error;

    private static string FormatMessage(Response response)
    {
        Guard.NotNull(response, nameof(response));
        LanguageError? error = response.Error;
        return error is null ? "language error" : error.Code + ": " + error.Message;
    }
}
