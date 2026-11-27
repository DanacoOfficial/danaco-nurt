// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Report.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Report.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System.Text.Json;

namespace Danaco.Nurt;

/// <summary>Effect report <c>nurt-report/1</c> of one program.</summary>
/// <remarks>
/// The properties cover the fields used to drive a client. Every other field of the
/// report is available through <see cref="Element"/>.
/// </remarks>
public sealed class Report
{
    internal Report(JsonElement element)
    {
        Element = element;
        StatusText = JsonRead.String(element, "status") ?? string.Empty;
        Status = ParseStatus(StatusText);
        Kind = JsonRead.String(element, "kind") ?? string.Empty;
        Operation = JsonRead.Int64(element, "op");
        NextCursor = JsonRead.String(element, "next_cursor");
        JsonElement? error = JsonRead.Property(element, "error");
        Error = error is { ValueKind: JsonValueKind.Object } value ? new LanguageError(value) : null;
    }

    /// <summary>Outcome of the program.</summary>
    public ReportStatus Status { get; }

    /// <summary>Outcome of the program as written in the report, for example <c>rolled_back</c>.</summary>
    public string StatusText { get; }

    /// <summary>Kind of operation, for example <c>replace</c> or <c>find</c>.</summary>
    public string Kind { get; }

    /// <summary>Number of the journal entry of the operation (<c>op</c>); <see langword="null"/> when no entry was written.</summary>
    public long? Operation { get; }

    /// <summary>Cursor of the next page; <see langword="null"/> on the last page.</summary>
    public string? NextCursor { get; }

    /// <summary>Error of the language; <see langword="null"/> when the program raised none.</summary>
    public LanguageError? Error { get; }

    /// <summary>The whole report as JSON.</summary>
    public JsonElement Element { get; }

    internal static ReportStatus ParseStatus(string value) => value switch
    {
        "ok" => ReportStatus.Ok,
        "planned" => ReportStatus.Planned,
        "running" => ReportStatus.Running,
        "partial" => ReportStatus.Partial,
        "rolled_back" => ReportStatus.RolledBack,
        "failed" => ReportStatus.Failed,
        _ => ReportStatus.Unknown,
    };

    internal static Report Read(JsonElement response)
    {
        JsonElement? report = JsonRead.Property(response, "report");
        if (report is not { ValueKind: JsonValueKind.Object } value)
        {
            throw new NurtCallException(CallStatus.Internal, "response has no report object");
        }

        return new Report(value);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
