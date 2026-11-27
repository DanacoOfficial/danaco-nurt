// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/src/Danaco.Nurt/Internal/TaskWait.cs
// Responsibility:   Source file bindings/dotnet/src/Danaco.Nurt/Internal/TaskWait.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt;

internal static class TaskWait
{
    /// <summary>
    /// Waits for a task without observing its result. Returns <see langword="false"/> when
    /// the timeout elapses first and throws <see cref="OperationCanceledException"/> when the
    /// token is cancelled first.
    /// </summary>
    public static async Task<bool> WaitAsync(Task task, TimeSpan? timeout, CancellationToken cancellationToken)
    {
        if (task.IsCompleted)
        {
            return true;
        }

        cancellationToken.ThrowIfCancellationRequested();
        if (timeout is null && !cancellationToken.CanBeCanceled)
        {
            await Observe(task).ConfigureAwait(false);
            return true;
        }

        using var stop = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        Task delay = Task.Delay(timeout ?? Timeout.InfiniteTimeSpan, stop.Token);
        Task first = await Task.WhenAny(task, delay).ConfigureAwait(false);
        stop.Cancel();
        if (first == task)
        {
            return true;
        }

        cancellationToken.ThrowIfCancellationRequested();
        return false;
    }

    private static Task Observe(Task task) =>
        task.ContinueWith(
            static _ => { },
            CancellationToken.None,
            TaskContinuationOptions.ExecuteSynchronously,
            TaskScheduler.Default);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
