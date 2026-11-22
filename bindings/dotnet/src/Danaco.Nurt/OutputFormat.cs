namespace Danaco.Nurt;

/// <summary>Format of the rendered result.</summary>
public enum OutputFormat
{
    /// <summary>Plain text (<c>text</c>).</summary>
    Text = 0,

    /// <summary>The report only, without rendered text (<c>json</c>).</summary>
    Json = 1,

    /// <summary>Markdown (<c>markdown</c>).</summary>
    Markdown = 2,

    /// <summary>Short text for constrained contexts (<c>compact</c>).</summary>
    Compact = 3,
}
