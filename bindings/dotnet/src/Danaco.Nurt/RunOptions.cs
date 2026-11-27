// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/RunOptions.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/RunOptions.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;
using System.Collections.Generic;

namespace Danaco.Nurt;

/// <summary>
/// Call options of a request envelope. An option left <see langword="null"/> takes its
/// value from the configuration or the built-in default.
/// </summary>
public sealed class RunOptions
{
    /// <summary>Only computes targets and effect; changes nothing (<c>preview</c>).</summary>
    public bool? Preview { get; init; }

    /// <summary>Absolute directory against which relative paths are resolved (<c>base</c>).</summary>
    public string? Base { get; init; }

    /// <summary>Records or changes per page, from 1 to 5000 (<c>limit</c>).</summary>
    public int? Limit { get; init; }

    /// <summary>Fields of the result to keep (<c>fields</c>).</summary>
    public IReadOnlyList<string>? Fields { get; init; }

    /// <summary>Format of the rendered result (<c>format</c>).</summary>
    public OutputFormat? Format { get; init; }

    /// <summary>Amount of detail of the rendered result (<c>verbosity</c>).</summary>
    public Verbosity? Verbosity { get; init; }

    /// <summary>Form of the differences in the report (<c>diff</c>).</summary>
    public DiffMode? Diff { get; init; }

    /// <summary>Lines of context around each difference (<c>diff_context</c>).</summary>
    public int? DiffContext { get; init; }

    /// <summary>Largest number of characters of a single text field; 0 means no limit (<c>max_chars</c>).</summary>
    public int? MaxChars { get; init; }

    /// <summary>Absolute path of a file that receives the full result instead of the response (<c>save_to</c>).</summary>
    public string? SaveTo { get; init; }

    /// <summary>Time limit of the whole program (<c>timeout</c>); sent in whole milliseconds.</summary>
    public TimeSpan? Timeout { get; init; }

    /// <summary>Session identifier recorded in the journal (<c>session</c>); overrides <see cref="NurtClientOptions.Session"/>.</summary>
    public string? Session { get; init; }

    /// <summary>Language of messages and help, <c>en</c> or <c>pl</c> (<c>language</c>); overrides <see cref="NurtClientOptions.Language"/>.</summary>
    public string? Language { get; init; }

    /// <summary>Script mode: several pipelines, each its own operation (<c>script</c>).</summary>
    public bool? Script { get; init; }

    /// <summary>
    /// Raises <see cref="NurtLanguageException"/> when the response carries an error of the
    /// language, instead of returning the response. Not sent to Danaco Nurt.
    /// </summary>
    public bool ThrowOnLanguageError { get; init; }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
