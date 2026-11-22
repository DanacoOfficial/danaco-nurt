using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>
/// Result of binding and classifying a request without running it: bound tree, targets,
/// paths, external programs, network addresses, corrections and notable effects.
/// </summary>
public sealed class CheckResult
{
    internal CheckResult(JsonElement result, JsonElement? diagnostics)
    {
        Result = result;
        Diagnostics = diagnostics;
    }

    /// <summary>Classification of the request as JSON, in the format <c>nurt-check/1</c>.</summary>
    public JsonElement Result { get; }

    /// <summary>Diagnostics of binding as JSON; <see langword="null"/> when there are none.</summary>
    public JsonElement? Diagnostics { get; }
}
