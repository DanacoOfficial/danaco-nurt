using System;
using System.IO;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt;

/// <summary>
/// Backend that reaches Danaco Nurt through a supervised child process
/// <c>nurt serve --stdio</c>. The process is started on the first call and started again
/// by the next call after it exits.
/// </summary>
internal sealed class ProcessBackend : IBackend
{
    private readonly string _executable;
    private readonly ClientOptions _options;
    private readonly SemaphoreSlim _connectLock = new(1, 1);
    private RpcConnection? _connection;
    private int _disposed;

    public ProcessBackend(string executable, ClientOptions options)
    {
        _executable = executable;
        _options = options;
    }

    public BackendKind Kind => BackendKind.Process;

    /// <summary>Identifier of the running child process; <see langword="null"/> before the first call.</summary>
    internal int? ProcessId => Volatile.Read(ref _connection) is { IsAlive: true } connection ? connection.ProcessId : null;

    public async Task<Response> RunAsync(byte[] envelope, CancellationToken cancellationToken) =>
        Response.FromElement(await CallAsync("run", envelope, cancellationToken).ConfigureAwait(false));

    public async Task<IExecutionHandle> StartAsync(byte[] envelope, EventQueue events, CancellationToken cancellationToken)
    {
        RpcConnection connection = await ConnectAsync(cancellationToken).ConfigureAwait(false);
        PendingCall call = await connection.BeginAsync("run", envelope, events, useSlot: true, cancellationToken).ConfigureAwait(false);
        return new ProcessExecution(connection, call, events);
    }

    public async Task<CheckResult> CheckAsync(byte[] envelope, CancellationToken cancellationToken)
    {
        JsonElement result = await CallAsync("check", envelope, cancellationToken).ConfigureAwait(false);
        return new CheckResult(result, JsonRead.Property(result, "diagnostics"));
    }

    public async Task<ParseResult> ParseAsync(string program, NotationVariant variant, CancellationToken cancellationToken)
    {
        var parameters = new SyntaxParams { Program = program, Variant = WireFormat.VariantName(variant) };
        JsonElement result = await CallAsync("parse", Serialize(parameters), cancellationToken).ConfigureAwait(false);
        return new ParseResult(JsonRead.Property(result, "ast"), JsonRead.Property(result, "diagnostics"), JsonRead.Property(result, "error"));
    }

    public async Task<string> PrintAsync(JsonElement ast, NotationVariant variant, CancellationToken cancellationToken)
    {
        var parameters = new SyntaxParams { Ast = ast, Variant = WireFormat.VariantName(variant) };
        return Text(await CallAsync("print", Serialize(parameters), cancellationToken).ConfigureAwait(false), "print");
    }

    public async Task<CorrectResult> CorrectAsync(string program, NotationVariant variant, CancellationToken cancellationToken)
    {
        var parameters = new SyntaxParams { Program = program, Variant = WireFormat.VariantName(variant) };
        return new CorrectResult(await CallAsync("correct", Serialize(parameters), cancellationToken).ConfigureAwait(false));
    }

    public async Task<Response> PageAsync(string cursor, int? limit, CancellationToken cancellationToken)
    {
        byte[] parameters = JsonSerializer.SerializeToUtf8Bytes(new PageParams { Cursor = cursor, Limit = limit }, WireJsonContext.Default.PageParams);
        return Response.FromElement(await CallAsync("page", parameters, cancellationToken).ConfigureAwait(false));
    }

    public async Task<string> DescribeAsync(DescribeParams selector, CancellationToken cancellationToken)
    {
        byte[] parameters = JsonSerializer.SerializeToUtf8Bytes(selector, WireJsonContext.Default.DescribeParams);
        return Text(await CallAsync("describe", parameters, cancellationToken).ConfigureAwait(false), "describe");
    }

    public async Task<string> HelpAsync(string? topic, CancellationToken cancellationToken)
    {
        byte[] parameters = JsonSerializer.SerializeToUtf8Bytes(new HelpParams { Topic = topic }, WireJsonContext.Default.HelpParams);
        return Text(await CallAsync("help", parameters, cancellationToken).ConfigureAwait(false), "help");
    }

    public Task<JsonElement> CatalogAsync(JsonElement? what, CancellationToken cancellationToken) =>
        CallAsync("catalog", Named("what", what), cancellationToken);

    public Task<JsonElement> SchemaAsync(string which, CancellationToken cancellationToken) =>
        CallAsync("schema", JsonSerializer.SerializeToUtf8Bytes(new SchemaParams { Which = which }, WireJsonContext.Default.SchemaParams), cancellationToken);

    public Task<JsonElement> HistoryAsync(JsonElement? query, CancellationToken cancellationToken) =>
        CallAsync("history", Named("query", query), cancellationToken);

    public Task<JsonElement> SpaceAsync(string? volume, CancellationToken cancellationToken) =>
        CallAsync("space", JsonSerializer.SerializeToUtf8Bytes(new SpaceParams { Volume = volume }, WireJsonContext.Default.SpaceParams), cancellationToken);

    public Task<JsonElement> InfoAsync(CancellationToken cancellationToken) =>
        CallAsync("info", null, cancellationToken);

    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) != 0)
        {
            return;
        }

        Interlocked.Exchange(ref _connection, null)?.Close(_options.ShutdownTimeout);
        _connectLock.Dispose();
    }

    public ValueTask DisposeAsync() => new(Task.Run(Dispose));

    private static byte[] Serialize(SyntaxParams parameters) =>
        JsonSerializer.SerializeToUtf8Bytes(parameters, WireJsonContext.Default.SyntaxParams);

    /// <summary>Writes the parameters <c>{"name": value}</c>; no parameters when the value is absent.</summary>
    private static byte[]? Named(string name, JsonElement? value)
    {
        if (value is not { } present)
        {
            return null;
        }

        using var stream = new MemoryStream();
        using (var writer = new Utf8JsonWriter(stream))
        {
            writer.WriteStartObject();
            writer.WritePropertyName(name);
            present.WriteTo(writer);
            writer.WriteEndObject();
        }

        return stream.ToArray();
    }

    /// <summary>Reads a text result, given either as a JSON string or as an object with the member <c>text</c>.</summary>
    private static string Text(JsonElement result, string method) => result.ValueKind == JsonValueKind.String
        ? result.GetString()!
        : JsonRead.String(result, "text") ?? throw new CallException(CallStatus.Internal, method + ": result carries no text");

    private async Task<JsonElement> CallAsync(string method, byte[]? parameters, CancellationToken cancellationToken)
    {
        RpcConnection connection = await ConnectAsync(cancellationToken).ConfigureAwait(false);
        return await connection.CallAsync(method, parameters, useSlot: true, cancellationToken).ConfigureAwait(false);
    }

    private async Task<RpcConnection> ConnectAsync(CancellationToken cancellationToken)
    {
        if (Volatile.Read(ref _connection) is { IsAlive: true } current)
        {
            return current;
        }

        Guard.NotDisposed(Volatile.Read(ref _disposed) != 0, this);
        await _connectLock.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            Guard.NotDisposed(Volatile.Read(ref _disposed) != 0, this);
            if (_connection is { IsAlive: true } existing)
            {
                return existing;
            }

            _connection?.Close(TimeSpan.Zero);
            _connection = null;
            RpcConnection started = await RpcConnection.OpenAsync(_executable, _options, cancellationToken).ConfigureAwait(false);
            Volatile.Write(ref _connection, started);
            return started;
        }
        finally
        {
            _connectLock.Release();
        }
    }
}
