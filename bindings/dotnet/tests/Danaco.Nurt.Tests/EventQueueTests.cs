// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

[TestClass]
public sealed class EventQueueTests
{
    [TestMethod]
    public async Task EventsAreReadInOrderUntilFinished()
    {
        var queue = new EventQueue();
        Task<List<long>> reader = ReadSequencesAsync(queue);

        queue.Add(Event("job.output", 1));
        queue.Add(Event("job.progress", 2));
        queue.Add(Event("exec.finished", 3));
        queue.Add(Event("job.output", 4));

        CollectionAssert.AreEqual(new List<long> { 1, 2, 3 }, await reader);
    }

    [TestMethod]
    public async Task CompleteEndsReadingAfterQueuedEvents()
    {
        var queue = new EventQueue();
        queue.Add(Event("job.output", 1));
        queue.Complete();

        CollectionAssert.AreEqual(new List<long> { 1 }, await ReadSequencesAsync(queue));
    }

    [TestMethod]
    public async Task SecondReaderIsRejected()
    {
        var queue = new EventQueue();
        queue.Complete();
        await ReadSequencesAsync(queue);

        await TestSupport.ThrowsAsync<InvalidOperationException>(() => ReadSequencesAsync(queue));
    }

    [TestMethod]
    public async Task ReadingStopsOnCancellation()
    {
        var queue = new EventQueue();
        using var cancellation = new CancellationTokenSource(TimeSpan.FromMilliseconds(50));

        await TestSupport.ThrowsAsync<OperationCanceledException>(async () =>
        {
            await foreach (NurtExecutionEvent item in queue.ReadAllAsync(cancellation.Token))
            {
                Assert.Fail("unexpected event " + item.Kind);
            }
        });
    }

    [TestMethod]
    public void EventFieldsAreRead()
    {
        NurtExecutionEvent item = NurtExecutionEvent.Parse("""{"t": "job.output", "seq": 7, "time": "2026-01-01T00:00:00Z", "id": "j1", "lines": ["a"]}""");

        Assert.AreEqual("job.output", item.Kind);
        Assert.AreEqual(7, item.Sequence);
        Assert.AreEqual("2026-01-01T00:00:00Z", item.Time);
        Assert.AreEqual("j1", item.JobId);
        Assert.IsFalse(item.IsFinished);
        Assert.AreEqual("a", item.Element.GetProperty("lines")[0].GetString());
    }

    private static NurtExecutionEvent Event(string kind, long sequence) =>
        NurtExecutionEvent.Parse("{\"t\": \"" + kind + "\", \"seq\": " + sequence.ToString(System.Globalization.CultureInfo.InvariantCulture) + "}");

    private static async Task<List<long>> ReadSequencesAsync(EventQueue queue)
    {
        var sequences = new List<long>();
        await foreach (NurtExecutionEvent item in queue.ReadAllAsync(CancellationToken.None))
        {
            sequences.Add(item.Sequence);
        }

        return sequences;
    }
}
