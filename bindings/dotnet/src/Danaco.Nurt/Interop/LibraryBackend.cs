// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.Collections.Concurrent;
using System.Globalization;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt;

/// <summary>
/// Backend that calls <c>libnurt</c> in the current process. Calls run on the thread pool;
/// an execution waits on a dedicated thread. Only the members that pass pointers are
/// unsafe, because an unsafe context cannot contain <see langword="await"/>.
/// </summary>
internal sealed class LibraryBackend : IBackend
{
    private readonly EngineHandle _engine;
    private readonly ConcurrentDictionary<LibraryExecution, bool> _executions = new();
    private int _disposed;

    private LibraryBackend(EngineHandle engine)
    {
        _engine = engine;
    }

    public BackendKind Kind => BackendKind.Library;

    /// <summary>
    /// Loads the library and checks its interface version: the major version must equal
    /// <see cref="NativeMethods.AbiMajor"/> and the minor version must not be lower than
    /// <see cref="NativeMethods.AbiMinor"/>.
    /// </summary>
    public static void Load(string path)
    {
        LibraryLoader.Load(path);
        uint version;
        try
        {
            version = NativeMethods.AbiVersion();
        }
        catch (DllNotFoundException exception)
        {
            throw new NurtCallException(CallStatus.Io, "cannot load libnurt from " + path + ": " + exception.Message, exception);
        }
        catch (EntryPointNotFoundException exception)
        {
            throw new NurtCallException(CallStatus.AbiMismatch, "library " + path + " does not export the interface of libnurt: " + exception.Message, exception);
        }
        catch (BadImageFormatException exception)
        {
            throw new NurtCallException(CallStatus.Io, "libnurt at " + path + " is built for another platform", exception);
        }

        int major = (int)(version >> NativeMethods.AbiMajorShift);
        int minor = (int)(version & NativeMethods.AbiMinorMask);
        if (major != NativeMethods.AbiMajor || minor < NativeMethods.AbiMinor)
        {
            throw new NurtCallException(
                CallStatus.AbiMismatch,
                string.Format(
                    CultureInfo.InvariantCulture,
                    "libnurt implements interface {0}.{1}; this client requires {2}.{3} or a later minor version",
                    major,
                    minor,
                    NativeMethods.AbiMajor,
                    NativeMethods.AbiMinor));
        }
    }

    /// <summary>Opens an engine of the loaded library with the configuration of the client.</summary>
    public static unsafe LibraryBackend Open(NurtClientOptions options)
    {
        byte[]? config = options.Config is { } value ? Utf8.Terminate(WireFormat.Compact(value)) : null;
        CallStatus status;
        EngineHandle engine;
        fixed (byte* pointer = config)
        {
            status = NativeMethods.EngineOpen(pointer, out engine);
        }

        if (status != CallStatus.Ok)
        {
            NurtCallException failure = NativeCall.Failure(status, "nurt_engine_open");
            engine.Dispose();
            throw failure;
        }

        return new LibraryBackend(engine);
    }

    public Task<NurtResponse> RunAsync(byte[] envelope, CancellationToken cancellationToken)
    {
        if (!cancellationToken.CanBeCanceled)
        {
            return Task.Run(() => NurtResponse.Parse(RunJson(envelope)), cancellationToken);
        }

        return RunCancellableAsync(envelope, cancellationToken);
    }

    public Task<IExecutionHandle> StartAsync(byte[] envelope, EventQueue events, CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        return Task.FromResult<IExecutionHandle>(new LibraryExecution(_engine, envelope, events, _executions));
    }

    public Task<CheckResult> CheckAsync(byte[] envelope, CancellationToken cancellationToken) =>
        Task.Run(() => CheckCore(envelope), cancellationToken);

    public Task<ParseResult> ParseAsync(string program, NotationVariant variant, CancellationToken cancellationToken)
    {
        byte[] text = Utf8.ToNative(program, nameof(program))!;
        int native = WireFormat.NativeVariant(variant);
        return Task.Run(() => ParseCore(text, native), cancellationToken);
    }

    public unsafe Task<string> PrintAsync(JsonElement ast, NotationVariant variant, CancellationToken cancellationToken)
    {
        byte[] tree = Utf8.Terminate(WireFormat.Compact(ast));
        int native = WireFormat.NativeVariant(variant);
        return Task.Run(() => NativeCall.Text(tree, "nurt_print", (input, output) => NativeMethods.Print(_engine, input, native, output)), cancellationToken);
    }

    public unsafe Task<CorrectResult> CorrectAsync(string program, NotationVariant variant, CancellationToken cancellationToken)
    {
        byte[] text = Utf8.ToNative(program, nameof(program))!;
        int native = WireFormat.NativeVariant(variant);
        return Task.Run(
            () => new CorrectResult(NativeCall.Json(text, "nurt_correct", (input, output) => NativeMethods.Correct(_engine, input, native, output))),
            cancellationToken);
    }

    public unsafe Task<NurtResponse> PageAsync(string cursor, int? limit, CancellationToken cancellationToken)
    {
        byte[] text = Utf8.ToNative(cursor, nameof(cursor))!;
        uint pageLimit = limit is { } value ? (uint)value : 0;
        return Task.Run(
            () => NurtResponse.Parse(NativeCall.Text(text, "nurt_page", (input, output) => NativeMethods.Page(_engine, input, pageLimit, output))),
            cancellationToken);
    }

    public unsafe Task<string> DescribeAsync(DescribeParams selector, CancellationToken cancellationToken)
    {
        byte[] what = Utf8.Terminate(JsonSerializer.SerializeToUtf8Bytes(selector, WireJsonContext.Default.DescribeParams));
        return Task.Run(() => NativeCall.Text(what, "nurt_describe", (input, output) => NativeMethods.Describe(_engine, input, output)), cancellationToken);
    }

    public unsafe Task<string> HelpAsync(string? topic, CancellationToken cancellationToken)
    {
        byte[]? text = Utf8.ToNative(topic, nameof(topic));
        return Task.Run(() => NativeCall.Text(text, "nurt_help", (input, output) => NativeMethods.Help(_engine, input, output)), cancellationToken);
    }

    public unsafe Task<JsonElement> CatalogAsync(JsonElement? what, CancellationToken cancellationToken)
    {
        byte[]? selector = what is { } value ? Utf8.Terminate(WireFormat.Compact(value)) : null;
        return Task.Run(() => NativeCall.Json(selector, "nurt_catalog", (input, output) => NativeMethods.Catalog(_engine, input, output)), cancellationToken);
    }

    public unsafe Task<JsonElement> SchemaAsync(string which, CancellationToken cancellationToken)
    {
        byte[] name = Utf8.ToNative(which, nameof(which))!;
        return Task.Run(() => NativeCall.Json(name, "nurt_schema", static (input, output) => NativeMethods.Schema(input, output)), cancellationToken);
    }

    public unsafe Task<JsonElement> HistoryAsync(JsonElement? query, CancellationToken cancellationToken)
    {
        byte[]? parameters = query is { } value ? Utf8.Terminate(WireFormat.Compact(value)) : null;
        return Task.Run(() => NativeCall.Json(parameters, "nurt_history", (input, output) => NativeMethods.History(_engine, input, output)), cancellationToken);
    }

    public unsafe Task<JsonElement> SpaceAsync(string? volume, CancellationToken cancellationToken)
    {
        byte[]? path = Utf8.ToNative(volume, nameof(volume));
        return Task.Run(() => NativeCall.Json(path, "nurt_space", (input, output) => NativeMethods.Space(_engine, input, output)), cancellationToken);
    }

    public unsafe Task<JsonElement> InfoAsync(CancellationToken cancellationToken) =>
        Task.Run(() => NativeCall.Json(null, "nurt_info", static (_, output) => NativeMethods.Info(output)), cancellationToken);

    /// <summary>
    /// Cancels and releases the executions that are still open, then closes the engine.
    /// The engine is closed when the last execution handle is released.
    /// </summary>
    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) != 0)
        {
            return;
        }

        foreach (LibraryExecution execution in _executions.Keys)
        {
            execution.Dispose();
        }

        _engine.Dispose();
    }

    public ValueTask DisposeAsync()
    {
        Dispose();
        return default;
    }

    private async Task<NurtResponse> RunCancellableAsync(byte[] envelope, CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        using var execution = new LibraryExecution(_engine, envelope, null, _executions);
        using CancellationTokenRegistration registration = cancellationToken.Register(execution.RequestCancel);
        NurtResponse response = await execution.Completion.ConfigureAwait(false);
        cancellationToken.ThrowIfCancellationRequested();
        return response;
    }

    private unsafe string RunJson(byte[] envelope)
    {
        byte[] request = Utf8.Terminate(envelope);
        NativeBuffer response = default;
        try
        {
            fixed (byte* pointer = request)
            {
                NativeCall.Check(NativeMethods.RunJson(_engine, pointer, &response), "nurt_run_json");
            }

            return response.ReadString();
        }
        finally
        {
            NativeMethods.BufferFree(&response);
        }
    }

    private unsafe CheckResult CheckCore(byte[] envelope)
    {
        byte[] request = Utf8.Terminate(envelope);
        NativeBuffer bound = default;
        NativeBuffer diagnostics = default;
        try
        {
            fixed (byte* pointer = request)
            {
                NativeCall.Check(NativeMethods.Check(_engine, pointer, &bound, &diagnostics), "nurt_check");
            }

            JsonElement result = bound.ReadJson("nurt_check result")
                ?? throw new NurtCallException(CallStatus.Internal, "nurt_check: result is empty");
            return new CheckResult(result, diagnostics.ReadJson("nurt_check diagnostics"));
        }
        finally
        {
            NativeMethods.BufferFree(&bound);
            NativeMethods.BufferFree(&diagnostics);
        }
    }

    private unsafe ParseResult ParseCore(byte[] program, int variant)
    {
        NativeBuffer ast = default;
        NativeBuffer diagnostics = default;
        try
        {
            fixed (byte* pointer = program)
            {
                NativeCall.Check(NativeMethods.Parse(_engine, pointer, variant, &ast, &diagnostics), "nurt_parse");
            }

            return new ParseResult(ast.ReadJson("nurt_parse tree"), diagnostics.ReadJson("nurt_parse diagnostics"), null);
        }
        finally
        {
            NativeMethods.BufferFree(&ast);
            NativeMethods.BufferFree(&diagnostics);
        }
    }
}
