// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt;

/// <summary>
/// Operations of one backend. Request envelopes and selectors are UTF-8 JSON without a
/// terminating zero byte.
/// </summary>
internal interface IBackend : IDisposable, IAsyncDisposable
{
    BackendKind Kind { get; }

    Task<NurtResponse> RunAsync(byte[] envelope, CancellationToken cancellationToken);

    Task<IExecutionHandle> StartAsync(byte[] envelope, EventQueue events, CancellationToken cancellationToken);

    Task<CheckResult> CheckAsync(byte[] envelope, CancellationToken cancellationToken);

    Task<ParseResult> ParseAsync(string program, NotationVariant variant, CancellationToken cancellationToken);

    Task<string> PrintAsync(JsonElement ast, NotationVariant variant, CancellationToken cancellationToken);

    Task<CorrectResult> CorrectAsync(string program, NotationVariant variant, CancellationToken cancellationToken);

    Task<NurtResponse> PageAsync(string cursor, int? limit, CancellationToken cancellationToken);

    Task<string> DescribeAsync(DescribeParams selector, CancellationToken cancellationToken);

    Task<string> HelpAsync(string? topic, CancellationToken cancellationToken);

    Task<JsonElement> CatalogAsync(JsonElement? what, CancellationToken cancellationToken);

    Task<JsonElement> SchemaAsync(string which, CancellationToken cancellationToken);

    Task<JsonElement> HistoryAsync(JsonElement? query, CancellationToken cancellationToken);

    Task<JsonElement> SpaceAsync(string? volume, CancellationToken cancellationToken);

    Task<JsonElement> InfoAsync(CancellationToken cancellationToken);
}
