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

    Task<Response> RunAsync(byte[] envelope, CancellationToken cancellationToken);

    Task<IExecutionHandle> StartAsync(byte[] envelope, EventQueue events, CancellationToken cancellationToken);

    Task<CheckResult> CheckAsync(byte[] envelope, CancellationToken cancellationToken);

    Task<ParseResult> ParseAsync(string program, NotationVariant variant, CancellationToken cancellationToken);

    Task<string> PrintAsync(JsonElement ast, NotationVariant variant, CancellationToken cancellationToken);

    Task<CorrectResult> CorrectAsync(string program, NotationVariant variant, CancellationToken cancellationToken);

    Task<Response> PageAsync(string cursor, int? limit, CancellationToken cancellationToken);

    Task<string> DescribeAsync(DescribeParams selector, CancellationToken cancellationToken);

    Task<string> HelpAsync(string? topic, CancellationToken cancellationToken);

    Task<JsonElement> CatalogAsync(JsonElement? what, CancellationToken cancellationToken);

    Task<JsonElement> SchemaAsync(string which, CancellationToken cancellationToken);

    Task<JsonElement> HistoryAsync(JsonElement? query, CancellationToken cancellationToken);

    Task<JsonElement> SpaceAsync(string? volume, CancellationToken cancellationToken);

    Task<JsonElement> InfoAsync(CancellationToken cancellationToken);
}
