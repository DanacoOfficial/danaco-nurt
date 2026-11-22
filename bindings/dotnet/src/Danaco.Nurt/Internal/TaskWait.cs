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
