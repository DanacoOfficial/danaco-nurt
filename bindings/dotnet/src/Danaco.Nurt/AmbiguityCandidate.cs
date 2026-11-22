using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>
/// One reading of an ambiguous program, listed by an <c>E_AMBIGUOUS</c> error. A candidate
/// is run with <see cref="Request.FromAccept(string)"/>.
/// </summary>
public sealed class AmbiguityCandidate
{
    internal AmbiguityCandidate(JsonElement element)
    {
        Rank = JsonRead.Int32(element, "rank") ?? 0;
        Ref = JsonRead.String(element, "ref");
        Program = JsonRead.String(element, "program") ?? string.Empty;
        Why = JsonRead.String(element, "why");
    }

    /// <summary>Rank of the candidate, from 1 for the most likely reading.</summary>
    public int Rank { get; }

    /// <summary>Identifier passed to <see cref="Request.FromAccept(string)"/>; <see langword="null"/> when not reported.</summary>
    public string? Ref { get; }

    /// <summary>Canonical text of the program read this way.</summary>
    public string Program { get; }

    /// <summary>Reason for the reading; <see langword="null"/> when not reported.</summary>
    public string? Why { get; }
}
