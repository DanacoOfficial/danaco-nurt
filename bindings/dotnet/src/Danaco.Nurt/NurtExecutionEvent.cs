// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>
/// Event of an execution <c>nurt-event/1</c>: progress, output of programs, journal
/// changes or completion.
/// </summary>
public sealed class NurtExecutionEvent
{
    /// <summary>Kind of the event that ends an execution.</summary>
    public const string FinishedKind = "exec.finished";

    private NurtExecutionEvent(JsonElement element)
    {
        Element = element;
        Kind = JsonRead.String(element, "t") ?? string.Empty;
        Sequence = JsonRead.Int64(element, "seq") ?? 0;
        Time = JsonRead.String(element, "time");
        JobId = Kind.StartsWith("job.", System.StringComparison.Ordinal)
            ? JsonRead.String(element, "id") ?? JsonRead.String(element, "job_id")
            : JsonRead.String(element, "job_id");
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

    internal static NurtExecutionEvent FromElement(JsonElement element) => new(element.Clone());

    internal static NurtExecutionEvent Parse(string json) => new(JsonRead.Parse(json, "event"));
}
