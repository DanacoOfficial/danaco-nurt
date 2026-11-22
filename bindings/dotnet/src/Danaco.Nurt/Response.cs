using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>
/// Response <c>nurt-response/1</c> to a request: the effect report and, for text formats,
/// the rendered text.
/// </summary>
/// <remarks>
/// An error of the language is not an exception: <see cref="Ok"/> is <see langword="false"/>
/// and <see cref="Error"/> describes it.
/// </remarks>
public sealed class Response
{
    private Response(string raw, JsonElement element)
    {
        Raw = raw;
        Element = element;
        Report = Report.Read(element);
        JsonElement? rendered = JsonRead.Property(element, "rendered");
        Text = rendered is { ValueKind: JsonValueKind.Object } value ? JsonRead.String(value, "text") : null;
    }

    /// <summary>Effect report of the program.</summary>
    public Report Report { get; }

    /// <summary>Text rendered from the report; <see langword="null"/> for the format <see cref="OutputFormat.Json"/>.</summary>
    public string? Text { get; }

    /// <summary><see langword="true"/> when the report carries no error of the language.</summary>
    public bool Ok => Report.Error is null;

    /// <summary>Error of the language; <see langword="null"/> when <see cref="Ok"/> is <see langword="true"/>.</summary>
    public LanguageError? Error => Report.Error;

    /// <summary>Cursor of the next page; <see langword="null"/> on the last page.</summary>
    public string? NextCursor => Report.NextCursor;

    /// <summary>The response as received, in JSON.</summary>
    public string Raw { get; }

    /// <summary>The response as a JSON element.</summary>
    public JsonElement Element { get; }

    internal static Response Parse(string json) => new(json, JsonRead.Parse(json, "response"));

    internal static Response FromElement(JsonElement element) => new(element.GetRawText(), element.Clone());

    internal Response ThrowIfRequested(RunOptions? options)
    {
        if (options is { ThrowOnLanguageError: true } && !Ok)
        {
            throw new LanguageException(this);
        }

        return this;
    }
}
