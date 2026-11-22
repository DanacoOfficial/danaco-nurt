using System;
using System.Collections.Generic;
using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>Settings of a <see cref="Client"/>.</summary>
public sealed class ClientOptions
{
    internal static readonly IReadOnlyList<string> DefaultServeArguments = new[] { "serve", "--stdio" };

    /// <summary>Mechanism through which the client reaches Danaco Nurt. Default: <see cref="BackendKind.Auto"/>.</summary>
    public BackendKind Backend { get; init; }

    /// <summary>
    /// Path of the <c>nurt</c> executable. When <see langword="null"/>, the variable
    /// <c>NURT_EXECUTABLE</c>, the file bundled with the package for the current platform
    /// and the directories of <c>PATH</c> are searched in this order.
    /// </summary>
    public string? ExecutablePath { get; init; }

    /// <summary>
    /// Path of the <c>libnurt</c> library. When <see langword="null"/>, the variable
    /// <c>NURT_LIBRARY</c> and the file bundled with the package for the current platform
    /// are searched in this order.
    /// </summary>
    public string? LibraryPath { get; init; }

    /// <summary>
    /// Configuration in the format <c>nurt-config/1</c>, applied on top of the
    /// configuration files; <see langword="null"/> for none.
    /// </summary>
    public JsonElement? Config { get; init; }

    /// <summary>Session identifier used by requests that do not set <see cref="RunOptions.Session"/>.</summary>
    public string? Session { get; init; }

    /// <summary>Language of messages and help, <c>en</c> or <c>pl</c>. Default: <c>en</c>.</summary>
    public string Language { get; init; } = "en";

    /// <summary>
    /// Time allowed for <c>nurt serve</c> to finish its operations and exit when the client
    /// is disposed, before the process is terminated. Default: 5 seconds.
    /// </summary>
    public TimeSpan ShutdownTimeout { get; init; } = TimeSpan.FromSeconds(5);

    internal IReadOnlyList<string> ServeArguments { get; init; } = DefaultServeArguments;
}
