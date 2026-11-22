using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>
/// Event of an execution <c>nurt-event/1</c>: progress, output of programs, journal
/// changes or completion.
/// </summary>
public sealed class ExecutionEvent
{
    /// <summary>Kind of the event that ends an execution.</summary>
    public const string FinishedKind = "exec.finished";

    private ExecutionEvent(JsonElement element)
    {
        Element = element;
        Kind = JsonRead.String(element, "t") ?? string.Empty;
        Sequence = JsonRead.Int64(element, "seq") ?? 0;
        Time = JsonRead.String(element, "time");
        JobId = JsonRead.String(element, "job_id");
    }

    /// <summary>Kind of the event (field <c>t</c>), for example <c>job.output</c> or <c>exec.finished</c>.</summary>
    public string Kind { get; }

    /// <summary>Sequence number; events of one execution arrive in increasing order.</summary>
    public long Sequence { get; }

    /// <summary>Time of the event in RFC 3339 form; <see langword="null"/> when not reported.</summary>
    public string? Time { get; }

    /// <summary>Identifier of the background job the event belongs to; <see langword="null"/> otherwise.</summary>
    public string? JobId { get; }

    /// <summary><see langword="true"/> for the last event of an execution.</summary>
    public bool IsFinished => Kind == FinishedKind;

    /// <summary>The whole event as JSON.</summary>
    public JsonElement Element { get; }

    internal static ExecutionEvent FromElement(JsonElement element) => new(element.Clone());

    internal static ExecutionEvent Parse(string json) => new(JsonRead.Parse(json, "event"));
}
