// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Diagnostics.CodeAnalysis;
using System.Globalization;
using System.IO;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt;

/// <summary>
/// One child process <c>nurt serve --stdio</c> and the JSON-RPC 2.0 session over its
/// standard streams: one message per line (NDJSON), requests matched to responses by
/// identifier, notifications <c>nurt/event</c> routed to executions by <c>request_id</c>.
/// </summary>
[SuppressMessage(
    "Design",
    "CA1001:Types that own disposable fields should be disposable",
    Justification = "The session ends with Close(TimeSpan), which waits for the process with the shutdown timeout of the owner before it releases the lock.")]
internal sealed class RpcConnection
{
    public const string EventMethod = "nurt/event";

    private const int StderrTailLines = 20;
    private const int ExitWaitMilliseconds = 2000;
    private static readonly UTF8Encoding NoBom = new(encoderShouldEmitUTF8Identifier: false);

    private readonly Process _process;
    private readonly Stream _input;
    private readonly StreamReader _output;
    private readonly SemaphoreSlim _writeLock = new(1, 1);
    private readonly ConcurrentDictionary<long, PendingCall> _pending = new();
    private readonly ConcurrentDictionary<long, EventQueue> _routes = new();
    private readonly Queue<string> _stderrTail = new();
    private readonly Task _readLoop;
    private readonly Task _stderrLoop;
    private SemaphoreSlim? _slots;
    private NurtCallException? _failure;
    private long _lastId;

    private RpcConnection(Process process)
    {
        _process = process;
        _input = process.StandardInput.BaseStream;
        _output = process.StandardOutput;
        _stderrLoop = Task.Run(ReadErrorsAsync);
        _readLoop = Task.Run(ReadLoopAsync);
    }

    public bool IsAlive => Volatile.Read(ref _failure) is null;

    public int ProcessId => _process.Id;

    public static async Task<RpcConnection> OpenAsync(string executable, NurtClientOptions options, CancellationToken cancellationToken)
    {
        var connection = new RpcConnection(Start(executable, options.ServeArguments));
        try
        {
            var parameters = new InitializeParams
            {
                Language = options.Language,
                Session = options.Session,
            };
            JsonElement result = await connection.CallAsync(
                "initialize",
                JsonSerializer.SerializeToUtf8Bytes(parameters, WireJsonContext.Default.InitializeParams),
                useSlot: false,
                cancellationToken).ConfigureAwait(false);
            connection.Accept(result);
            return connection;
        }
        catch
        {
            connection.Close(TimeSpan.Zero);
            throw;
        }
    }

    public async Task<JsonElement> CallAsync(string method, byte[]? parameters, bool useSlot, CancellationToken cancellationToken)
    {
        PendingCall call = await BeginAsync(method, parameters, null, useSlot, cancellationToken).ConfigureAwait(false);
        if (cancellationToken.CanBeCanceled)
        {
            using CancellationTokenRegistration registration = cancellationToken.Register(() => CancelInBackground(call.Id));
            await TaskWait.WaitAsync(call.Source.Task, null, cancellationToken).ConfigureAwait(false);
        }

        return await call.Source.Task.ConfigureAwait(false);
    }

    public async Task<PendingCall> BeginAsync(string method, byte[]? parameters, EventQueue? events, bool useSlot, CancellationToken cancellationToken)
    {
        ThrowIfFailed();
        SemaphoreSlim? slots = useSlot ? Volatile.Read(ref _slots) : null;
        try
        {
            if (slots is not null)
            {
                await slots.WaitAsync(cancellationToken).ConfigureAwait(false);
            }
        }
        catch (ObjectDisposedException)
        {
            ThrowIfFailed();
            throw;
        }

        var call = new PendingCall(Interlocked.Increment(ref _lastId), method, slots);
        _pending[call.Id] = call;
        if (events is not null)
        {
            _routes[call.Id] = events;
        }

        try
        {
            ThrowIfFailed();
            await WriteAsync(Frame(call.Id, method, parameters)).ConfigureAwait(false);
        }
        catch (IOException exception)
        {
            Abandon(call.Id);
            NurtCallException failure = Fail(new NurtCallException(CallStatus.Internal, "cannot write to nurt serve: " + exception.Message, exception));
            throw new NurtCallException(failure.Status, failure.Message, failure);
        }
        catch
        {
            Abandon(call.Id);
            throw;
        }

        return call;
    }

    /// <summary>Requests cancellation of a request in progress; does nothing when it has already ended.</summary>
    public async Task CancelRequestAsync(long requestId, CancellationToken cancellationToken)
    {
        if (!IsAlive || !_pending.ContainsKey(requestId))
        {
            return;
        }

        byte[] parameters = JsonSerializer.SerializeToUtf8Bytes(new CancelParams { RequestId = requestId }, WireJsonContext.Default.CancelParams);
        await CallAsync("cancel", parameters, useSlot: false, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>Requests cancellation without waiting; failures are ignored because the request ends either way.</summary>
    public void CancelInBackground(long requestId) =>
        _ = CancelRequestAsync(requestId, CancellationToken.None).ContinueWith(
            static task => _ = task.Exception,
            CancellationToken.None,
            TaskContinuationOptions.OnlyOnFaulted | TaskContinuationOptions.ExecuteSynchronously,
            TaskScheduler.Default);

    public void RemoveRoute(long requestId) => _routes.TryRemove(requestId, out _);

    /// <summary>
    /// Ends the session: closes the input of the process, which <c>nurt serve</c> treats as
    /// <c>shutdown</c>, waits for the exit and terminates the process after the timeout.
    /// </summary>
    public void Close(TimeSpan timeout)
    {
        Fail(new NurtCallException(CallStatus.Internal, "connection to nurt serve is closed"));
        try
        {
            _input.Dispose();
        }
        catch (IOException)
        {
            // The pipe is already broken when the process has exited; the exit is awaited below.
        }

        if (!_process.WaitForExit(Milliseconds(timeout)))
        {
            Kill();
        }

        Task.WaitAll(new[] { _readLoop, _stderrLoop }, ExitWaitMilliseconds);
        _process.Dispose();
        _writeLock.Dispose();
        Volatile.Read(ref _slots)?.Dispose();
    }

    private static Process Start(string executable, IReadOnlyList<string> arguments)
    {
#if NET8_0_OR_GREATER
        EnsureExecutable(executable);
#endif
        var info = new ProcessStartInfo(executable)
        {
            UseShellExecute = false,
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            CreateNoWindow = true,
            StandardOutputEncoding = NoBom,
            StandardErrorEncoding = NoBom,
        };
        CommandLine.AddArguments(info, arguments);
        var process = new Process { StartInfo = info };
        try
        {
            process.Start();
        }
        catch (Win32Exception exception)
        {
            process.Dispose();
            throw new NurtCallException(CallStatus.Io, "cannot start " + executable + ": " + exception.Message, exception);
        }

        return process;
    }

#if NET8_0_OR_GREATER
    /// <summary>
    /// Restores the execute permission of a file extracted from a package: NuGet does not
    /// keep Unix file modes when it extracts native assets. When the mode cannot be changed,
    /// starting the process reports the cause.
    /// </summary>
    private static void EnsureExecutable(string path)
    {
        if (OperatingSystem.IsWindows())
        {
            return;
        }

        const UnixFileMode Execute = UnixFileMode.UserExecute | UnixFileMode.GroupExecute | UnixFileMode.OtherExecute;
        try
        {
            UnixFileMode mode = File.GetUnixFileMode(path);
            if ((mode & UnixFileMode.UserExecute) == 0)
            {
                File.SetUnixFileMode(path, mode | Execute);
            }
        }
        catch (IOException)
        {
            return;
        }
        catch (UnauthorizedAccessException)
        {
            return;
        }
    }
#endif

    private static int Milliseconds(TimeSpan value) =>
        value <= TimeSpan.Zero ? 0 : (int)Math.Min(value.TotalMilliseconds, int.MaxValue);

    private static byte[] Frame(long id, string method, byte[]? parameters)
    {
        using var stream = new MemoryStream();
        using (var writer = new Utf8JsonWriter(stream))
        {
            writer.WriteStartObject();
            writer.WriteString("jsonrpc", "2.0");
            writer.WriteNumber("id", id);
            writer.WriteString("method", method);
            if (parameters is not null)
            {
                writer.WritePropertyName("params");
                writer.WriteRawValue(parameters, skipInputValidation: true);
            }

            writer.WriteEndObject();
        }

        stream.WriteByte((byte)'\n');
        return stream.ToArray();
    }

    private void Accept(JsonElement result)
    {
        string? protocol = JsonRead.String(result, "protocol");
        if (protocol is not null && protocol != WireFormat.Protocol)
        {
            throw new NurtCallException(CallStatus.AbiMismatch, "nurt serve uses protocol " + protocol + "; this client requires " + WireFormat.Protocol);
        }

        if (JsonRead.Int32(result, "abi") is { } version
            && version >> NativeMethods.AbiMajorShift is var major
            && major != NativeMethods.AbiMajor)
        {
            throw new NurtCallException(
                CallStatus.AbiMismatch,
                string.Format(CultureInfo.InvariantCulture, "nurt serve implements interface {0}.x; this client requires {1}.x", major, NativeMethods.AbiMajor));
        }

        if (JsonRead.Property(result, "limits") is { ValueKind: JsonValueKind.Object } limits
            && JsonRead.Int32(limits, "max_inflight") is > 0 and int maxInflight)
        {
            Volatile.Write(ref _slots, new SemaphoreSlim(maxInflight, maxInflight));
        }
    }

    private async Task WriteAsync(byte[] frame)
    {
        await _writeLock.WaitAsync().ConfigureAwait(false);
        try
        {
#if NET8_0_OR_GREATER
            await _input.WriteAsync(frame.AsMemory()).ConfigureAwait(false);
#else
            await _input.WriteAsync(frame, 0, frame.Length).ConfigureAwait(false);
#endif
            await _input.FlushAsync().ConfigureAwait(false);
        }
        finally
        {
            _writeLock.Release();
        }
    }

    private async Task ReadLoopAsync()
    {
        string reason = "nurt serve closed its output";
        try
        {
            while (await _output.ReadLineAsync().ConfigureAwait(false) is { } line)
            {
                if (line.Length != 0 && !Dispatch(line))
                {
                    reason = "nurt serve wrote a line that is not a JSON-RPC 2.0 message";
                    Kill();
                    break;
                }
            }
        }
        catch (IOException exception)
        {
            reason = "cannot read from nurt serve: " + exception.Message;
        }
        catch (ObjectDisposedException)
        {
        }

        Fail(new NurtCallException(CallStatus.Internal, DescribeExit(reason)));
    }

    private bool Dispatch(string line)
    {
        JsonDocument document;
        try
        {
            document = JsonDocument.Parse(line);
        }
        catch (JsonException)
        {
            return false;
        }

        using (document)
        {
            JsonElement message = document.RootElement;
            if (message.ValueKind != JsonValueKind.Object)
            {
                return false;
            }

            if (JsonRead.String(message, "method") is { } method)
            {
                if (method == EventMethod && JsonRead.Property(message, "params") is { ValueKind: JsonValueKind.Object } parameters)
                {
                    Route(parameters);
                }

                return true;
            }

            if (JsonRead.Int64(message, "id") is { } id && _pending.TryRemove(id, out PendingCall? call))
            {
                call.ReleaseSlot();
                if (JsonRead.Property(message, "error") is { ValueKind: JsonValueKind.Object } error)
                {
                    call.Source.TrySetException(RpcErrors.ToException(error, call.Method));
                }
                else
                {
                    JsonElement result = message.TryGetProperty("result", out JsonElement value) ? value.Clone() : default;
                    call.Source.TrySetResult(result);
                }

                if (_routes.TryRemove(id, out EventQueue? events))
                {
                    events.Complete();
                }
            }

            return true;
        }
    }

    private void Route(JsonElement parameters)
    {
        if (JsonRead.Int64(parameters, "request_id") is not { } requestId
            || !_routes.TryGetValue(requestId, out EventQueue? events))
        {
            return;
        }

        NurtExecutionEvent item = NurtExecutionEvent.FromElement(parameters);
        events.Add(item);
        if (item.IsFinished)
        {
            _routes.TryRemove(requestId, out _);
        }
    }

    private async Task ReadErrorsAsync()
    {
        try
        {
            StreamReader errors = _process.StandardError;
            while (await errors.ReadLineAsync().ConfigureAwait(false) is { } line)
            {
                lock (_stderrTail)
                {
                    _stderrTail.Enqueue(line);
                    if (_stderrTail.Count > StderrTailLines)
                    {
                        _stderrTail.Dequeue();
                    }
                }
            }
        }
        catch (IOException)
        {
        }
        catch (ObjectDisposedException)
        {
        }
    }

    private string DescribeExit(string reason)
    {
        var text = new StringBuilder(reason);
        try
        {
            if (_process.WaitForExit(ExitWaitMilliseconds))
            {
                text.Append("; exit code ").Append(_process.ExitCode.ToString(CultureInfo.InvariantCulture));
                _stderrLoop.Wait(ExitWaitMilliseconds);
            }
        }
        catch (InvalidOperationException)
        {
            // The process object was released by Close; the exit code is no longer needed.
        }

        lock (_stderrTail)
        {
            if (_stderrTail.Count > 0)
            {
                text.Append("; standard error: ").Append(string.Join(" | ", _stderrTail));
            }
        }

        return text.ToString();
    }

    private void ThrowIfFailed()
    {
        if (Volatile.Read(ref _failure) is { } failure)
        {
            throw new NurtCallException(failure.Status, failure.Message, failure);
        }
    }

    private NurtCallException Fail(NurtCallException failure)
    {
        NurtCallException effective = Interlocked.CompareExchange(ref _failure, failure, null) ?? failure;
        foreach (long id in _pending.Keys)
        {
            if (_pending.TryRemove(id, out PendingCall? call))
            {
                call.ReleaseSlot();
                call.Source.TrySetException(new NurtCallException(effective.Status, effective.Message, effective));
            }
        }

        foreach (long id in _routes.Keys)
        {
            if (_routes.TryRemove(id, out EventQueue? events))
            {
                events.Complete();
            }
        }

        return effective;
    }

    private void Abandon(long id)
    {
        if (_pending.TryRemove(id, out PendingCall? call))
        {
            call.ReleaseSlot();
        }

        _routes.TryRemove(id, out _);
    }

    private void Kill()
    {
        try
        {
#if NET8_0_OR_GREATER
            _process.Kill(entireProcessTree: true);
#else
            _process.Kill();
#endif
        }
        catch (InvalidOperationException)
        {
        }
        catch (Win32Exception)
        {
        }
    }
}
