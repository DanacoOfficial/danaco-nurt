using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>Result of correcting the written form of a program without running it.</summary>
public sealed class CorrectResult
{
    internal CorrectResult(JsonElement result)
    {
        Result = result;
        Program = JsonRead.String(result, "program");
    }

    /// <summary>Corrected program text; <see langword="null"/> when the program cannot be corrected.</summary>
    public string? Program { get; }

    /// <summary>The whole result as JSON, including the list of corrections.</summary>
    public JsonElement Result { get; }
}
