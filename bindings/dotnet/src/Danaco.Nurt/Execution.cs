// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Execution.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Execution.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt;

/// <summary>
/// Asynchronous execution started by <see cref="NurtClient.StartAsync(Request, RunOptions?, CancellationToken)"/>.
/// </summary>
/// <remarks>
/// Disposing an execution that is still running cancels it. Changes already committed stay
/// in the journal and can be undone.
/// </remarks>
public sealed class Execution : IDisposable, IAsyncDisposable
{
    private readonly IExecutionHandle _handle;
    private readonly EventQueue _events;
    private readonly RunOptions? _options;
    private int _disposed;

    internal Execution(IExecutionHandle handle, EventQueue events, RunOptions? options)
    {
        _handle = handle;
        _events = events;
        _options = options;
    }

    /// <summary><see langword="true"/> when the response is available.</summary>
    public bool IsCompleted => _handle.Completion.IsCompleted;

    /// <summary>Waits for the response of the execution.</summary>
    /// <param name="cancellationToken">Stops waiting; the execution continues.</param>
    /// <returns>The response; after <see cref="CancelAsync"/> the response of the cancelled execution.</returns>
    /// <exception cref="NurtCallException">The execution failed as a call.</exception>
    /// <exception cref="NurtLanguageException">The response carries an error and <see cref="RunOptions.ThrowOnLanguageError"/> is set.</exception>
    /// <exception cref="OperationCanceledException"><paramref name="cancellationToken"/> was cancelled.</exception>
    public Task<NurtResponse> WaitAsync(CancellationToken cancellationToken = default) =>
        WaitCoreAsync(null, cancellationToken);

    /// <summary>Waits for the response of the execution for at most the given time.</summary>
    /// <param name="timeout">Longest time to wait; <see cref="TimeSpan.Zero"/> checks without waiting.</param>
    /// <param name="cancellationToken">Stops waiting; the execution continues.</param>
    /// <returns>The response; after <see cref="CancelAsync"/> the response of the cancelled execution.</returns>
    /// <exception cref="NurtCallException">The execution is still running after <paramref name="timeout"/> (status <see cref="CallStatus.Timeout"/>), or it failed as a call.</exception>
    /// <exception cref="NurtLanguageException">The response carries an error and <see cref="RunOptions.ThrowOnLanguageError"/> is set.</exception>
    /// <exception cref="OperationCanceledException"><paramref name="cancellationToken"/> was cancelled.</exception>
    public Task<NurtResponse> WaitAsync(TimeSpan timeout, CancellationToken cancellationToken = default) =>
        WaitCoreAsync(timeout, cancellationToken);

    /// <summary>
    /// Requests cancellation of the execution. Idempotent; succeeds also when the execution
    /// has already finished.
    /// </summary>
    /// <param name="cancellationToken">Cancels sending the request.</param>
    /// <returns>A task that completes when the request has been delivered.</returns>
    /// <exception cref="NurtCallException">The request could not be delivered.</exception>
    public Task CancelAsync(CancellationToken cancellationToken = default)
    {
        Guard.NotDisposed(Volatile.Read(ref _disposed) != 0, this);
        return _handle.CancelAsync(cancellationToken);
    }

    /// <summary>
    /// Reads the events of the execution in the order of their sequence numbers, ending
    /// with <c>exec.finished</c>. The events can be read once.
    /// </summary>
    /// <param name="cancellationToken">Stops reading.</param>
    /// <returns>The events of the execution.</returns>
    /// <exception cref="InvalidOperationException">The events are already being read.</exception>
    public IAsyncEnumerable<NurtExecutionEvent> ReadEventsAsync(CancellationToken cancellationToken = default) =>
        _events.ReadAllAsync(cancellationToken);

    /// <summary>Cancels the execution when it is still running and releases it.</summary>
    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) == 0)
        {
            _handle.Dispose();
            _events.Complete();
        }

        GC.SuppressFinalize(this);
    }

    /// <summary>Cancels the execution when it is still running and releases it.</summary>
    /// <returns>A completed task.</returns>
    public ValueTask DisposeAsync()
    {
        Dispose();
        return default;
    }

    private async Task<NurtResponse> WaitCoreAsync(TimeSpan? timeout, CancellationToken cancellationToken)
    {
        Task<NurtResponse> completion = _handle.Completion;
        if (!await TaskWait.WaitAsync(completion, timeout, cancellationToken).ConfigureAwait(false))
        {
            throw new NurtCallException(CallStatus.Timeout, "execution is still running after the timeout; wait again or cancel it");
        }

        NurtResponse response = await completion.ConfigureAwait(false);
        return response.ThrowIfRequested(_options);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
