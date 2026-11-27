// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/tests/Danaco.Nurt.Tests/EngineTests.cs
// Responsibility:   Tests of the process backend against the real engine nurt serve --stdio.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

/// <summary>
/// Every method of the process backend on the real engine. The tests need the variable
/// <c>NURT_TEST_EXECUTABLE</c>, which names the <c>nurt</c> executable; without it they are inconclusive.
/// </summary>
[TestClass]
public sealed class EngineTests
{
    private const string ExecutableVariable = "NURT_TEST_EXECUTABLE";
    private const string StateVariable = "NURT_STATE_DIR";

    private string? _directory;
    private string? _previousState;

    [TestInitialize]
    public void Prepare()
    {
        _directory = Path.Combine(Path.GetTempPath(), "nurt-engine-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(Path.Combine(_directory, "files"));
        _previousState = Environment.GetEnvironmentVariable(StateVariable);
        Environment.SetEnvironmentVariable(StateVariable, Path.Combine(_directory, "state"));
    }

    [TestCleanup]
    public void Clean()
    {
        Environment.SetEnvironmentVariable(StateVariable, _previousState);
        if (_directory is not null && Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }
    }

    private static NurtClient Engine()
    {
        string? executable = TestSupport.Variable(ExecutableVariable);
        if (executable is null)
        {
            Assert.Inconclusive("no engine: set " + ExecutableVariable + " to the nurt executable");
        }

        return new NurtClient(new NurtClientOptions { Backend = BackendKind.Process, ExecutablePath = executable });
    }

    private string Listing(int files)
    {
        string directory = Path.Combine(_directory!, "files");
        for (int index = 0; index < files; index++)
        {
            File.WriteAllText(Path.Combine(directory, "file-" + index + ".txt"), "text\n");
        }

        return "list \"" + directory.Replace('\\', '/') + "\"";
    }

    [TestMethod]
    public async Task InfoReportsTheFormats()
    {
        await using NurtClient client = Engine();

        JsonElement info = await client.InfoAsync();

        Assert.AreEqual("1.1", info.GetProperty("formats").GetProperty("ast").GetString());
        Assert.AreEqual("1.1", info.GetProperty("formats").GetProperty("bound").GetString());
        Assert.AreEqual(JsonValueKind.String, info.GetProperty("version").ValueKind);
    }

    [TestMethod]
    public async Task CatalogSelectionsAreAnswered()
    {
        await using NurtClient client = Engine();

        Assert.AreEqual("nurt-catalog-result/1", (await client.CatalogAsync()).GetProperty("format").GetString());
        JsonElement group = await client.CatalogAsync(JsonDocument.Parse("{\"group\":\"core\"}").RootElement);
        Assert.IsTrue(group.GetProperty("commands").GetArrayLength() > 0);
        JsonElement command = await client.CatalogAsync(JsonDocument.Parse("{\"command\":\"list\"}").RootElement);
        Assert.AreEqual("list", command.GetProperty("command").GetProperty("name").GetString());
    }

    [TestMethod]
    public async Task SchemaIsServedAndAnUnknownOneIsAnArgumentError()
    {
        await using NurtClient client = Engine();

        Assert.AreEqual("nurt-ast/1", (await client.SchemaAsync("ast")).GetProperty("title").GetString());
        NurtCallException failure = await TestSupport.ThrowsAsync<NurtCallException>(() => client.SchemaAsync("unknown"));
        Assert.AreEqual(CallStatus.InvalidArgument, failure.Status);
    }

    [TestMethod]
    public async Task DescribeAndHelpReturnText()
    {
        await using NurtClient client = Engine();

        string description = await client.DescribeAsync();
        StringAssert.Contains(description, "PROGRAM");
        Assert.IsFalse(description.StartsWith('{'));
        NurtLanguageException unknown = await TestSupport.ThrowsAsync<NurtLanguageException>(() => client.DescribeAsync("core", "no-such-command"));
        Assert.AreEqual("E_UNKNOWN_NAME", unknown.Error!.Code);
        StringAssert.StartsWith(await client.HelpAsync(), "GROUP");
        Assert.IsFalse(string.IsNullOrEmpty(await client.HelpAsync("list")));
    }

    [TestMethod]
    public async Task AShortenedResultCarriesAContinuation()
    {
        await using NurtClient client = Engine();

        NurtResponse response = await client.RunAsync(Listing(5), new RunOptions { Limit = 2 });

        Assert.IsTrue(response.Ok, response.Raw);
        Assert.AreEqual("list", response.Report.Kind);
        JsonElement continuation = response.Report.Element.GetProperty("continuation");
        Assert.AreEqual(3, continuation.GetProperty("remaining").GetInt32());
        NurtResponse next = await client.RunAsync("next");
        Assert.IsTrue(next.Ok, next.Raw);
        Assert.AreEqual("next", next.Report.Kind);
        Assert.IsTrue((await client.PageAsync(response.NextCursor!)).Ok);
    }

    [TestMethod]
    public async Task ALanguageErrorIsPartOfTheResponse()
    {
        await using NurtClient client = Engine();

        NurtResponse response = await client.RunAsync("lst \"/tmp\"");

        Assert.IsFalse(response.Ok);
        Assert.AreEqual("E_UNKNOWN_COMMAND", response.Error!.Code);
        Assert.AreEqual("E_ARG", (await client.PageAsync("malformed")).Error!.Code);
        await TestSupport.ThrowsAsync<NurtLanguageException>(() => client.RunAsync("lst \"/tmp\"", new RunOptions { ThrowOnLanguageError = true }));
    }

    [TestMethod]
    public async Task EventsPrecedeTheResponse()
    {
        await using NurtClient client = Engine();
        await using Execution execution = await client.StartAsync(Listing(1));
        var kinds = new List<string>();

        await foreach (NurtExecutionEvent item in execution.ReadEventsAsync())
        {
            kinds.Add(item.Kind);
        }

        Assert.IsTrue((await execution.WaitAsync()).Ok);
        Assert.AreEqual("exec.started", kinds[0]);
        Assert.AreEqual("exec.finished", kinds[^1]);
    }

    [TestMethod]
    public async Task CheckParsePrintAndCorrectReturnTheResults()
    {
        await using NurtClient client = Engine();

        CheckResult check = await client.CheckAsync(Request.FromProgram("list \"/tmp\""));
        Assert.IsNull(check.Error);
        Assert.IsTrue(check.Result.TryGetProperty("bound", out _));
        ParseResult parsed = await client.ParseAsync("list \"/tmp\"");
        Assert.IsNull(parsed.Error);
        Assert.AreEqual("1.1", parsed.Ast!.Value.GetProperty("nurt").GetString());
        StringAssert.StartsWith(await client.PrintAsync(parsed.Ast.Value), "list");
        ParseResult broken = await client.ParseAsync("list (");
        Assert.AreEqual("E_SYNTAX", broken.Error!.Code);
        Assert.AreEqual("list \"/tmp\"", (await client.CorrectAsync("list \"/tmp\"")).Program);
        CorrectResult quoted = await client.CorrectAsync("find '/tmp'");
        Assert.AreEqual("find \"/tmp\"", quoted.Program);
        Assert.IsTrue(quoted.Result.GetProperty("corrections").GetArrayLength() > 0);
    }

    [TestMethod]
    public async Task HistoryAndSpaceAreAnswered()
    {
        await using NurtClient client = Engine();

        Assert.AreEqual(JsonValueKind.Array, (await client.HistoryAsync()).GetProperty("operations").ValueKind);
        JsonElement query = JsonDocument.Parse("{\"limit\":3}").RootElement;
        Assert.AreEqual(JsonValueKind.Array, (await client.HistoryAsync(query)).GetProperty("operations").ValueKind);
        JsonElement text = JsonDocument.Parse("\"{\\\"limit\\\":3}\"").RootElement;
        Assert.AreEqual(JsonValueKind.Array, (await client.HistoryAsync(text)).GetProperty("operations").ValueKind);
        Assert.AreEqual(JsonValueKind.Array, (await client.SpaceAsync()).GetProperty("volumes").ValueKind);
    }

    [TestMethod]
    public async Task TheOptionsPrivilegeAndAsUserAreAccepted()
    {
        await using NurtClient client = Engine();
        string program = Listing(1);

        foreach (Privilege privilege in new[] { Privilege.Account, Privilege.Auto })
        {
            Assert.IsTrue((await client.RunAsync(program, new RunOptions { Privilege = privilege })).Ok);
        }

        NurtResponse response = await client.RunAsync(program, new RunOptions { AsUser = "root" });
        if (response.Ok)
        {
            Assert.AreEqual("root", response.Report.Element.GetProperty("as_user").GetString());
        }
        else
        {
            StringAssert.StartsWith(response.Error!.Code, "E_");
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
