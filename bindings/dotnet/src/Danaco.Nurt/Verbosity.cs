namespace Danaco.Nurt;

/// <summary>Amount of detail of the rendered result.</summary>
public enum Verbosity
{
    /// <summary>Only the outcome (<c>quiet</c>).</summary>
    Quiet = 0,

    /// <summary>Outcome and a sample of changes (<c>normal</c>).</summary>
    Normal = 1,

    /// <summary>All details (<c>detailed</c>).</summary>
    Detailed = 2,
}
