// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/tests/Danaco.Nurt.Tests/LibraryBackendTests.cs
// Responsibility:   Source file bindings/dotnet/tests/Danaco.Nurt.Tests/LibraryBackendTests.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

/// <summary>
/// Library backend on a build of <c>libnurt</c> named by <c>NURT_TEST_LIBRARY</c>, for example
/// the test build of the crate <c>nurt-ffi</c>. The programs <c>sleep 10ms</c> and
/// <c>sleep 60s</c> are those of the binding cases: a short execution and one that runs until
/// it is cancelled.
/// </summary>
[TestClass]
public sealed class LibraryBackendTests
{
    private const string ShortProgram = "sleep 10ms";
    private const string LongProgram = "sleep 60s";
    private const int ExecutionCount = 8;
    private const int ThreadCount = 32;

    private static readonly TimeSpan Limit = TimeSpan.FromSeconds(30);

    [TestMethod]
    public void LibraryImplementsACompatibleInterfaceVersion()
    {
        LibraryBackend.Load(LibraryPath());

        uint version = NativeMethods.AbiVersion();

        Assert.AreEqual(NativeMethods.AbiMajor, (int)(version >> NativeMethods.AbiMajorShift));
        Assert.IsTrue((int)(version & NativeMethods.AbiMinorMask) >= NativeMethods.AbiMinor);
    }

    [TestMethod]
    public async Task InfoNamesTheVersion()
    {
        await using NurtClient client = LibraryClient();

        JsonElement info = await client.InfoAsync();

        Assert.AreEqual(BackendKind.Library, client.Backend);
        Assert.AreEqual(JsonValueKind.String, info.GetProperty("version").ValueKind);
    }

    [TestMethod]
    public async Task AutoBackendUsesACompatibleLibrary()
    {
        await using var client = new NurtClient(new NurtClientOptions { Backend = BackendKind.Auto, LibraryPath = LibraryPath() });

        Assert.AreEqual(BackendKind.Library, client.Backend);
    }

    [TestMethod]
    public async Task FinishedExecutionsReleaseTheirHandles()
    {
        await using NurtClient client = LibraryClient();
        int before = ExecHandle.LiveCount;

        for (int index = 0; index < ExecutionCount; index++)
        {
            await using Execution execution = await client.StartAsync(ShortProgram);
            NurtResponse response = await execution.WaitAsync(Limit);
            Assert.IsNotNull(response.Report);
        }

        Assert.IsTrue(await TestSupport.EventuallyAsync(() => Task.FromResult(ExecHandle.LiveCount == before), Limit));
    }

    [TestMethod]
    public async Task DisposingARunningExecutionReleasesItsHandle()
    {
        await using NurtClient client = LibraryClient();
        int before = ExecHandle.LiveCount;

        Execution execution = await client.StartAsync(LongProgram);
        Assert.AreEqual(before + 1, ExecHandle.LiveCount);
        await execution.DisposeAsync();

        Assert.IsTrue(await TestSupport.EventuallyAsync(() => Task.FromResult(ExecHandle.LiveCount == before), Limit));
    }

    [TestMethod]
    public async Task DisposingTheClientReleasesOpenExecutions()
    {
        int before = ExecHandle.LiveCount;
        NurtClient client = LibraryClient();
        var executions = new List<Execution>();
        try
        {
            executions.Add(await client.StartAsync(LongProgram));
            executions.Add(await client.StartAsync(LongProgram));

            await client.DisposeAsync();

            Assert.IsTrue(await TestSupport.EventuallyAsync(() => Task.FromResult(ExecHandle.LiveCount == before), Limit));
        }
        finally
        {
            foreach (Execution execution in executions)
            {
                await execution.DisposeAsync();
            }

            await client.DisposeAsync();
        }
    }

    [TestMethod]
    public async Task CancellationTokenCancelsARun()
    {
        await using NurtClient client = LibraryClient();
        int before = ExecHandle.LiveCount;
        using var cancellation = new CancellationTokenSource(TimeSpan.FromMilliseconds(200));

        await TestSupport.ThrowsAsync<OperationCanceledException>(() => client.RunAsync(LongProgram, null, cancellation.Token));

        Assert.IsTrue(await TestSupport.EventuallyAsync(() => Task.FromResult(ExecHandle.LiveCount == before), Limit));
    }

    [TestMethod]
    public async Task WaitTimeoutLeavesTheExecutionRunningUntilCancelled()
    {
        await using NurtClient client = LibraryClient();
        await using Execution execution = await client.StartAsync(LongProgram);

        NurtCallException timeout = await TestSupport.ThrowsAsync<NurtCallException>(() => execution.WaitAsync(TimeSpan.FromMilliseconds(50)));
        await execution.CancelAsync();
        NurtResponse response = await execution.WaitAsync(Limit);

        Assert.AreEqual(CallStatus.Timeout, timeout.Status);
        Assert.AreEqual("E_CANCELLED", response.Error?.Code);
    }

    [TestMethod]
    public async Task ParallelRunsFromManyThreadsComplete()
    {
        await using NurtClient client = LibraryClient();
        int before = ExecHandle.LiveCount;

        NurtResponse[] responses = await Task.WhenAll(Enumerable.Range(0, ThreadCount).Select(_ => Task.Run(() => client.RunAsync(ShortProgram))));

        Assert.AreEqual(ThreadCount, responses.Length);
        Assert.IsTrue(responses.All(response => response.Report.Element.ValueKind == JsonValueKind.Object));
        Assert.AreEqual(before, ExecHandle.LiveCount);
    }

    private static string LibraryPath() =>
        TestSupport.Variable(TestSupport.LibraryVariable)
        ?? throw new AssertInconclusiveException(TestSupport.LibraryVariable + " does not name a build of libnurt");

    private static NurtClient LibraryClient() =>
        new(new NurtClientOptions { Backend = BackendKind.Library, LibraryPath = LibraryPath() });
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
