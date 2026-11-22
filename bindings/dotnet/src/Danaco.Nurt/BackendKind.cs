namespace Danaco.Nurt;

/// <summary>Mechanism through which a <see cref="Client"/> reaches Danaco Nurt.</summary>
public enum BackendKind
{
    /// <summary>
    /// The library when it is found and implements a compatible interface version;
    /// otherwise the process.
    /// </summary>
    Auto = 0,

    /// <summary>The shared library <c>libnurt</c>, loaded into the current process.</summary>
    Library = 1,

    /// <summary>A child process <c>nurt serve --stdio</c>, reached through JSON-RPC 2.0.</summary>
    Process = 2,
}
