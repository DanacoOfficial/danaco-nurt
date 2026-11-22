using System;
using System.Collections.Generic;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt;

/// <summary>
/// Unbounded queue of the events of one execution with a single asynchronous reader.
/// Writers never block, so the native callback thread and the protocol reader are not
/// held up by a slow consumer.
/// </summary>
internal sealed class EventQueue
{
    private readonly object _gate = new();
    private readonly Queue<ExecutionEvent> _items = new();
    private TaskCompletionSource<bool>? _waiter;
    private bool _completed;
    private int _readerStarted;

    public void Add(ExecutionEvent item)
    {
        TaskCompletionSource<bool>? waiter;
        lock (_gate)
        {
            if (_completed)
            {
                return;
            }

            _items.Enqueue(item);
            _completed = item.IsFinished;
            waiter = _waiter;
            _waiter = null;
        }

        waiter?.TrySetResult(true);
    }

    public void Complete()
    {
        TaskCompletionSource<bool>? waiter;
        lock (_gate)
        {
            _completed = true;
            waiter = _waiter;
            _waiter = null;
        }

        waiter?.TrySetResult(true);
    }

    public async IAsyncEnumerable<ExecutionEvent> ReadAllAsync([EnumeratorCancellation] CancellationToken cancellationToken)
    {
        if (Interlocked.Exchange(ref _readerStarted, 1) != 0)
        {
            throw new InvalidOperationException("events of an execution can be read only once");
        }

        while (true)
        {
            ExecutionEvent? item = null;
            Task? wait = null;
            lock (_gate)
            {
                if (_items.Count > 0)
                {
                    item = _items.Dequeue();
                }
                else if (!_completed)
                {
                    _waiter ??= new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
                    wait = _waiter.Task;
                }
            }

            if (item is not null)
            {
                yield return item;
            }
            else if (wait is not null)
            {
                await TaskWait.WaitAsync(wait, null, cancellationToken).ConfigureAwait(false);
            }
            else
            {
                yield break;
            }
        }
    }
}
