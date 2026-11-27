// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Interop/LibraryExecution.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Interop/LibraryExecution.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;
using System.Collections.Concurrent;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt;

/// <summary>
/// Execution started with <c>nurt_exec_start</c>. A dedicated thread waits in
/// <c>nurt_exec_wait</c> and completes <see cref="Completion"/>; the handle is released after
/// that wait returns, because <see cref="SafeHandle"/> defers the release while a call uses it.
/// </summary>
internal sealed unsafe class LibraryExecution : IExecutionHandle
{
    private readonly ExecHandle _handle;
    private readonly EventQueue? _events;
    private readonly ConcurrentDictionary<LibraryExecution, bool> _registry;
    private int _disposed;

    /// <summary>Starts an execution and keeps it in <paramref name="registry"/> until it is disposed.</summary>
    public LibraryExecution(EngineHandle engine, byte[] envelope, EventQueue? events, ConcurrentDictionary<LibraryExecution, bool> registry)
    {
        _events = events;
        _registry = registry;
        byte[] request = Utf8.Terminate(envelope);

        // The reference on the engine is taken before the execution starts and is handed to the
        // execution handle, so that the engine cannot be closed between the start and the
        // handle taking its own reference.
        bool referenced = false;
        engine.DangerousAddRef(ref referenced);
        GCHandle state = events is null ? default : GCHandle.Alloc(events);
        CallStatus status;
        ExecHandle handle;
        try
        {
            fixed (byte* pointer = request)
            {
                status = NativeMethods.ExecStart(
                    engine,
                    pointer,
                    events is null ? IntPtr.Zero : EventCallback.Pointer,
                    events is null ? IntPtr.Zero : GCHandle.ToIntPtr(state),
                    out handle);
            }
        }
        catch
        {
            ReleaseStart(engine, state);
            throw;
        }

        if (status != CallStatus.Ok)
        {
            NurtCallException failure = NativeCall.Failure(status, "nurt_exec_start");
            handle.Dispose();
            ReleaseStart(engine, state);
            throw failure;
        }

        handle.Attach(state, engine);
        _handle = handle;
        registry[this] = true;
        Completion = Task.Factory.StartNew(
            WaitForResponse,
            CancellationToken.None,
            TaskCreationOptions.LongRunning,
            TaskScheduler.Default);
    }

    public Task<NurtResponse> Completion { get; }

    public Task CancelAsync(CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        Guard.NotDisposed(Volatile.Read(ref _disposed) != 0, this);
        NativeCall.Check(NativeMethods.ExecCancel(_handle), "nurt_exec_cancel");
        return Task.CompletedTask;
    }

    /// <summary>Requests cancellation from a cancellation token callback; a disposed execution is already cancelled.</summary>
    public void RequestCancel()
    {
        if (Volatile.Read(ref _disposed) != 0)
        {
            return;
        }

        try
        {
            _ = NativeMethods.ExecCancel(_handle);
        }
        catch (ObjectDisposedException)
        {
            return;
        }
    }

    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposed, 1) != 0)
        {
            return;
        }

        if (!Completion.IsCompleted)
        {
            _ = NativeMethods.ExecCancel(_handle);
        }

        _registry.TryRemove(this, out _);
        _handle.Dispose();
        _events?.Complete();
    }

    private static void ReleaseStart(EngineHandle engine, GCHandle state)
    {
        if (state.IsAllocated)
        {
            state.Free();
        }

        engine.DangerousRelease();
    }

    private NurtResponse WaitForResponse()
    {
        NativeBuffer response = default;
        try
        {
            CallStatus status = NativeMethods.ExecWait(_handle, NativeMethods.WaitForever, &response);
            if (status is CallStatus.Ok or CallStatus.Cancelled)
            {
                return NurtResponse.Parse(response.ReadString());
            }

            throw NativeCall.Failure(status, "nurt_exec_wait");
        }
        finally
        {
            NativeMethods.BufferFree(&response);
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
