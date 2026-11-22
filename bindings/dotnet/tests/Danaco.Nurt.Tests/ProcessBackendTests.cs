using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

[TestClass]
public sealed class ProcessBackendTests
{
    private static readonly TimeSpan BackgroundLimit = TimeSpan.FromSeconds(10);
    private static readonly string[] EventKinds = { "job.output", "job.output", "job.output", "exec.finished" };

    [TestMethod]
    public async Task RunReturnsReportAndRenderedText()
    {
        await using Client client = TestSupport.ProcessClient();

        Response response = await client.RunAsync("echo hello");

        Assert.AreEqual(BackendKind.Process, client.Backend);
        Assert.IsTrue(response.Ok);
        Assert.AreEqual(ReportStatus.Ok, response.Report.Status);
        Assert.AreEqual("echo", response.Report.Kind);
        Assert.AreEqual("hello", response.Text);
        Assert.IsNull(response.NextCursor);
    }

    [TestMethod]
    public async Task EnvelopeCarriesOptionsAndClientDefaults()
    {
        await using Client client = TestSupport.ProcessClient();
        var options = new RunOptions
        {
            Preview = true,
            Limit = 5,
            Timeout = TimeSpan.FromSeconds(2),
            Format = OutputFormat.Markdown,
        };

        Response response = await client.RunAsync(Request.FromProgram("echo x", NotationVariant.A), options);

        JsonElement request = response.Element.GetProperty("request");
        Assert.AreEqual("1.0", request.GetProperty("nurt").GetString());
        Assert.AreEqual("a", request.GetProperty("variant").GetString());
        JsonElement sent = request.GetProperty("options");
        Assert.IsTrue(sent.GetProperty("preview").GetBoolean());
        Assert.AreEqual(5, sent.GetProperty("limit").GetInt32());
        Assert.AreEqual("2000ms", sent.GetProperty("timeout").GetString());
        Assert.AreEqual("markdown", sent.GetProperty("format").GetString());
        Assert.AreEqual("tests", sent.GetProperty("session").GetString());
        Assert.AreEqual("en", sent.GetProperty("language").GetString());
        Assert.IsFalse(sent.TryGetProperty("script", out _));
    }

    [TestMethod]
    public async Task LanguageErrorIsReturnedWithoutException()
    {
        await using Client client = TestSupport.ProcessClient();

        Response response = await client.RunAsync("error");

        Assert.IsFalse(response.Ok);
        Assert.AreEqual(ReportStatus.Failed, response.Report.Status);
        LanguageError error = response.Error!;
        Assert.AreEqual("E_SYNTAX", error.Code);
        Assert.AreEqual("unexpected token", error.Message);
        Assert.AreEqual("1", error.Stage);
        Assert.AreEqual(1, error.Line);
        Assert.AreEqual(1, error.Column);
        Assert.AreEqual(5, error.Length);
        Assert.AreEqual("echo error", error.Fix[0]);
        Assert.AreEqual("syntax", error.Help);
        Assert.AreEqual("k1", error.Candidates[0].Ref);
        Assert.AreEqual(1, error.Candidates[0].Rank);
    }

    [TestMethod]
    public async Task ThrowOnLanguageErrorRaisesLanguageException()
    {
        await using Client client = TestSupport.ProcessClient();

        LanguageException exception = await TestSupport.ThrowsAsync<LanguageException>(
            () => client.RunAsync("error", new RunOptions { ThrowOnLanguageError = true }));

        Assert.AreEqual("E_SYNTAX", exception.Error!.Code);
        StringAssert.Contains(exception.Message, "E_SYNTAX");
    }

    [TestMethod]
    public async Task ProtocolErrorBecomesCallExceptionWithHeaderStatus()
    {
        await using Client client = TestSupport.ProcessClient();

        CallException exception = await TestSupport.ThrowsAsync<CallException>(() => client.RunAsync("fail 3"));

        Assert.AreEqual(CallStatus.InvalidArgument, exception.Status);
        Assert.AreEqual("NURT_E_ARG", exception.StatusName);
        Assert.AreEqual(-32003, exception.RpcErrorCode);
        StringAssert.Contains(exception.Detail, "test");
    }

    [TestMethod]
    public async Task UnknownSchemaIsInvalidArgument()
    {
        await using Client client = TestSupport.ProcessClient();

        CallException exception = await TestSupport.ThrowsAsync<CallException>(() => client.SchemaAsync("unknown"));

        Assert.AreEqual(CallStatus.InvalidArgument, exception.Status);
    }

    [TestMethod]
    public async Task CrashFailsTheCallAndTheNextCallStartsANewProcess()
    {
        await using Client client = TestSupport.ProcessClient();
        int first = await TestSupport.ServerProcessIdAsync(client);

        CallException exception = await TestSupport.ThrowsAsync<CallException>(() => client.RunAsync("crash"));
        Response again = await client.RunAsync("echo again");
        int second = await TestSupport.ServerProcessIdAsync(client);

        Assert.AreEqual(CallStatus.Internal, exception.Status);
        StringAssert.Contains(exception.Message, "exit code 3");
        StringAssert.Contains(exception.Message, "fatal: test crash");
        Assert.IsTrue(again.Ok);
        Assert.AreNotEqual(first, second);
    }

    [TestMethod]
    public async Task LineThatIsNotJsonEndsTheSession()
    {
        await using Client client = TestSupport.ProcessClient();

        CallException exception = await TestSupport.ThrowsAsync<CallException>(() => client.RunAsync("garbage"));
        Response again = await client.RunAsync("echo again");

        Assert.AreEqual(CallStatus.Internal, exception.Status);
        StringAssert.Contains(exception.Message, "not a JSON-RPC 2.0 message");
        Assert.IsTrue(again.Ok);
    }

    [TestMethod]
    public async Task CancellationTokenCancelsTheRequestInTheServer()
    {
        await using Client client = TestSupport.ProcessClient();
        using var cancellation = new CancellationTokenSource(TimeSpan.FromMilliseconds(200));

        await TestSupport.ThrowsAsync<OperationCanceledException>(() => client.RunAsync("sleep", null, cancellation.Token));

        Assert.IsTrue(await TestSupport.EventuallyAsync(async () => await TestSupport.CancelledCountAsync(client) == 1, BackgroundLimit));
    }

    [TestMethod]
    public async Task WaitTimeoutLeavesTheExecutionRunningUntilCancelled()
    {
        await using Client client = TestSupport.ProcessClient();
        await using Execution execution = await client.StartAsync("sleep");

        CallException timeout = await TestSupport.ThrowsAsync<CallException>(() => execution.WaitAsync(TimeSpan.FromMilliseconds(100)));
        Assert.AreEqual(CallStatus.Timeout, timeout.Status);
        Assert.IsFalse(execution.IsCompleted);

        await execution.CancelAsync();
        Response response = await execution.WaitAsync(BackgroundLimit);

        Assert.AreEqual("E_CANCELLED", response.Error!.Code);
        await execution.CancelAsync();
    }

    [TestMethod]
    public async Task EventsArriveInOrderAndEndWithFinished()
    {
        await using Client client = TestSupport.ProcessClient();
        await using Execution execution = await client.StartAsync("events 3");
        var kinds = new List<string>();
        long last = 0;

        await foreach (ExecutionEvent item in execution.ReadEventsAsync())
        {
            Assert.IsTrue(item.Sequence > last);
            last = item.Sequence;
            kinds.Add(item.Kind);
        }

        Response response = await execution.WaitAsync();
        CollectionAssert.AreEqual(EventKinds, kinds);
        Assert.IsTrue(response.Ok);
    }

    [TestMethod]
    public async Task EventsCanBeReadOnlyOnce()
    {
        await using Client client = TestSupport.ProcessClient();
        await using Execution execution = await client.StartAsync("events 1");
        await foreach (ExecutionEvent item in execution.ReadEventsAsync())
        {
            Assert.IsNotNull(item.Kind);
        }

        await TestSupport.ThrowsAsync<InvalidOperationException>(async () =>
        {
            await foreach (ExecutionEvent item in execution.ReadEventsAsync())
            {
                Assert.Fail("unexpected event " + item.Kind);
            }
        });
    }

    [TestMethod]
    public async Task ParallelCallsFromManyThreadsReceiveTheirOwnResponses()
    {
        const int Count = 64;
        await using Client client = TestSupport.ProcessClient();
        string[] programs = Enumerable.Range(0, Count)
            .Select(index => index % 2 == 0
                ? "echo " + index.ToString(CultureInfo.InvariantCulture)
                : "delay " + (index % 7 * 10).ToString(CultureInfo.InvariantCulture))
            .ToArray();

        Response[] responses = await Task.WhenAll(programs.Select(program => Task.Run(() => client.RunAsync(program))));

        for (int index = 0; index < Count; index++)
        {
            string expected = index % 2 == 0 ? programs[index]["echo ".Length..] : programs[index];
            Assert.AreEqual(expected, responses[index].Text);
        }
    }

    [TestMethod]
    public async Task MaxInflightLimitsRequestsInProgress()
    {
        await using Client client = TestSupport.ProcessClient("--max-inflight", "2");

        await Task.WhenAll(Enumerable.Range(0, 8).Select(_ => client.RunAsync("delay 100")));
        JsonElement info = await client.InfoAsync();

        int observed = info.GetProperty("max_inflight_observed").GetInt32();
        Assert.IsTrue(observed is >= 1 and <= 2, "observed " + observed.ToString(CultureInfo.InvariantCulture));
    }

    [TestMethod]
    public async Task DisposeEndsTheChildProcess()
    {
        Client client = TestSupport.ProcessClient();
        int processId = await TestSupport.ServerProcessIdAsync(client);

        await client.DisposeAsync();

        Assert.IsTrue(TestSupport.HasExited(processId));
        await TestSupport.ThrowsAsync<ObjectDisposedException>(() => client.RunAsync("echo x"));
    }

    [TestMethod]
    public async Task DisposingARunningExecutionCancelsIt()
    {
        await using Client client = TestSupport.ProcessClient();
        Execution execution = await client.StartAsync("sleep");

        execution.Dispose();

        Assert.IsTrue(await TestSupport.EventuallyAsync(async () => await TestSupport.CancelledCountAsync(client) == 1, BackgroundLimit));
    }

    [TestMethod]
    public async Task OtherProtocolIsAbiMismatch()
    {
        await using Client client = TestSupport.ProcessClient("--protocol", "nurt-rpc/2");

        CallException exception = await TestSupport.ThrowsAsync<CallException>(() => client.InfoAsync());

        Assert.AreEqual(CallStatus.AbiMismatch, exception.Status);
    }

    [TestMethod]
    public async Task PagesFollowTheCursor()
    {
        await using Client client = TestSupport.ProcessClient();

        Response first = await client.RunAsync("page");
        Response second = await client.PageAsync(first.NextCursor!, 10);
        Response byRequest = await client.RunAsync(Request.FromCursor("c1"));
        Response expired = await client.PageAsync("old");

        Assert.AreEqual("c1", first.NextCursor);
        Assert.AreEqual("page 2", second.Text);
        Assert.IsTrue(byRequest.Ok);
        Assert.AreEqual("E_EXPIRED_CANDIDATE", expired.Error!.Code);
        await TestSupport.ThrowsAsync<ArgumentOutOfRangeException>(() => client.PageAsync("c1", 0));
    }

    [TestMethod]
    public async Task ReadingMethodsReturnTheirResults()
    {
        await using Client client = TestSupport.ProcessClient();
        JsonElement selector = TestSupport.Json("""{"group": "documents"}""");

        Assert.AreEqual("description core pl", await client.DescribeAsync("core", null, "pl"));
        Assert.AreEqual("help journal", await client.HelpAsync("journal"));
        Assert.AreEqual("help index", await client.HelpAsync());
        Assert.AreEqual("report", (await client.SchemaAsync("report")).GetProperty("title").GetString());
        Assert.AreEqual("documents", (await client.CatalogAsync(selector)).GetProperty("what").GetProperty("group").GetString());
        Assert.AreEqual("documents", (await client.HistoryAsync(selector)).GetProperty("query").GetProperty("group").GetString());
        Assert.AreEqual("/home/user", (await client.SpaceAsync("/home/user")).GetProperty("volume").GetString());
        Assert.IsNotNull((await client.ParseAsync("echo x")).Ast);
        Assert.AreEqual("echo printed", await client.PrintAsync(TestSupport.Json("""{"nurt": "1.0", "program": []}""")));
        Assert.AreEqual("echo corrected", (await client.CorrectAsync("ehco x")).Program);
        CheckResult check = await client.CheckAsync(Request.FromProgram("echo x"));
        Assert.IsTrue(check.Result.TryGetProperty("bound", out _));
        Assert.IsNotNull(check.Diagnostics);
    }
}
