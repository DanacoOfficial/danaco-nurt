// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

/// <summary>
/// Binding cases of <c>conformance/bindings</c> on both backends, and the mapping of the
/// runner checked against the scripted server.
/// </summary>
[TestClass]
public sealed class ConformanceCaseTests
{
    /// <summary>
    /// Cases in the format of <c>conformance/bindings</c> written for the scripted server:
    /// every call, outcome, placeholder and kind of expectation of the format occurs at least once.
    /// </summary>
    private const string ScriptedCases = """
        {
          "format": "nurt-binding-cases/1",
          "cases": [
            {
              "id": "scripted.run-ok",
              "summary": "A response with rendered text.",
              "fs": [{ "path": "src/a.txt", "type": "file", "content_utf8": "a\n" }, { "path": "empty", "type": "dir" }],
              "steps": [
                {
                  "call": "run",
                  "args": { "program": "echo ${base}", "options": { "base": "${base}", "format": "markdown", "limit": 5, "timeout": "1m30s" } },
                  "expect": {
                    "outcome": "ok",
                    "fields": { "/ok": true, "/text": "${base}", "/report/kind": "echo", "/report/status": "ok" },
                    "absent": ["/error", "/next_cursor"]
                  }
                }
              ]
            },
            {
              "id": "scripted.language-error",
              "summary": "A language error is a response.",
              "steps": [
                {
                  "call": "run",
                  "args": { "program": "error" },
                  "expect": { "outcome": "language_error", "code": "E_SYNTAX", "fields": { "/ok": false, "/error/at/stage": "1" } }
                },
                {
                  "call": "run",
                  "args": { "program": "error", "raise_on_error": true },
                  "expect": { "outcome": "language_error", "code": "E_SYNTAX", "fields": { "/report/status": "failed" } }
                }
              ]
            },
            {
              "id": "scripted.call-errors",
              "summary": "Call errors carry the names of the header statuses.",
              "steps": [
                { "call": "run", "args": { "options": { "limit": 1 } }, "expect": { "outcome": "call_error", "status": "NURT_E_ARG" } },
                { "call": "run", "args": { "program": "fail 5" }, "expect": { "outcome": "call_error", "status": "NURT_E_IO" } },
                { "call": "schema", "args": { "which": "unknown" }, "expect": { "outcome": "call_error", "status": "NURT_E_ARG" } }
              ]
            },
            {
              "id": "scripted.page",
              "summary": "The cursor of the first page is passed to page through a placeholder.",
              "steps": [
                { "call": "run", "args": { "program": "page" }, "expect": { "outcome": "ok", "fields": { "/next_cursor": "c1" } } },
                {
                  "call": "page",
                  "args": { "cursor": "${step1/next_cursor}", "limit": 1 },
                  "expect": { "outcome": "ok", "fields": { "/text": "page 2", "/ok": true } }
                },
                {
                  "call": "run",
                  "args": { "program": "echo from ${step1/report/kind}" },
                  "expect": { "outcome": "ok", "fields": { "/text": "from page" } }
                }
              ]
            },
            {
              "id": "scripted.execution",
              "summary": "Events of an execution are read after waiting for it.",
              "steps": [
                { "call": "start", "args": { "program": "events 2" }, "expect": { "outcome": "ok" } },
                { "call": "wait", "execution": 1, "args": { "timeout_ms": 10000 }, "expect": { "outcome": "ok", "fields": { "/ok": true } } },
                {
                  "call": "events",
                  "execution": 1,
                  "expect": {
                    "outcome": "ok",
                    "fields": { "/value/0/t": "job.output", "/value/1/seq": 2, "/value/2/t": "exec.finished" },
                    "present": ["/value/0/time"]
                  }
                }
              ]
            },
            {
              "id": "scripted.cancel",
              "summary": "A cancelled execution ends with E_CANCELLED.",
              "steps": [
                { "call": "start", "args": { "program": "sleep" }, "expect": { "outcome": "ok" } },
                { "call": "cancel", "execution": 1, "expect": { "outcome": "ok" } },
                { "call": "wait", "execution": 1, "args": { "timeout_ms": 10000 }, "expect": { "outcome": "language_error", "code": "E_CANCELLED" } }
              ]
            },
            {
              "id": "scripted.reading-calls",
              "summary": "Calls without effect return their result views.",
              "steps": [
                { "call": "check", "args": { "program": "echo x" }, "expect": { "outcome": "ok", "present": ["/value/bound"] } },
                { "call": "parse", "args": { "program": "echo x", "variant": "b" }, "expect": { "outcome": "ok", "fields": { "/value/ast/nurt": "1.0" } } },
                { "call": "parse", "args": { "program": "error" }, "expect": { "outcome": "language_error", "code": "E_LEXICAL", "absent": ["/value/ast"] } },
                { "call": "print", "args": { "ast": { "nurt": "1.0", "program": [] } }, "expect": { "outcome": "ok", "fields": { "/value": "echo printed" } } },
                { "call": "correct", "args": { "program": "ehco x" }, "expect": { "outcome": "ok", "fields": { "/value/program": "echo corrected" } } },
                { "call": "describe", "args": { "language": "pl" }, "expect": { "outcome": "ok", "fields": { "/value": "description core pl" } } },
                { "call": "help", "args": { "topic": "find" }, "expect": { "outcome": "ok", "fields": { "/value": "help find" } } },
                { "call": "catalog", "args": { "what": { "group": "documents" } }, "expect": { "outcome": "ok", "fields": { "/value/what/group": "documents" } } },
                { "call": "schema", "args": { "which": "report" }, "expect": { "outcome": "ok", "fields": { "/value/title": "report" } } },
                { "call": "history", "args": { "query": { "last": "5" } }, "expect": { "outcome": "ok", "fields": { "/value/query/last": "5" } } },
                { "call": "space", "args": { "volume": "/home/user" }, "expect": { "outcome": "ok", "fields": { "/value/volume": "/home/user" } } },
                { "call": "info", "expect": { "outcome": "ok", "present": ["/value/version"] } }
              ]
            },
            {
              "id": "scripted.close",
              "summary": "A call after close is a call error.",
              "steps": [
                { "call": "close", "expect": { "outcome": "ok" } },
                { "call": "info", "expect": { "outcome": "call_error", "status": "NURT_E_ARG" } }
              ]
            },
            {
              "id": "scripted.library-only",
              "summary": "A case limited to the library backend is skipped on the process backend.",
              "backends": ["library"],
              "steps": [{ "call": "info", "expect": { "outcome": "ok" } }]
            }
          ]
        }
        """;

    private const string MismatchCase = """
        {
          "id": "scripted.mismatch",
          "summary": "An expectation that does not hold.",
          "steps": [
            { "call": "run", "args": { "program": "echo hello" }, "expect": { "outcome": "ok", "fields": { "/text": "other" }, "absent": ["/report"] } }
          ]
        }
        """;

    /// <summary>Set by the test framework; receives the skipped cases and the failed optional cases.</summary>
    public TestContext TestContext { get; set; } = null!;

    [TestMethod]
    public void CaseFilesUseOnlyCallsArgumentsAndExpectationsOfTheRunner()
    {
        List<JsonElement> cases = BindingCaseRunner.LoadCases(TestSupport.RequireRepositoryFile("conformance", "bindings"));

        List<string> problems = cases.SelectMany(testCase => BindingCaseRunner.Validate(testCase)).ToList();

        Assert.IsTrue(cases.Count > 0, "conformance/bindings holds no cases");
        Assert.AreEqual(0, problems.Count, string.Join(Environment.NewLine, problems));
    }

    [TestMethod]
    public async Task RunnerMapsEveryCallOnTheScriptedServer()
    {
        NurtClientOptions options = TestSupport.ProcessOptions(BackendKind.Process);
        var runner = new BindingCaseRunner(BackendKind.Process, () => options);

        List<BindingCaseRunner.CaseResult> results = await RunAllAsync(runner, BindingCaseRunner.CasesOf(TestSupport.Json(ScriptedCases)));

        string[] failures = results.Where(result => result.Failure is not null).Select(Describe).ToArray();
        Assert.AreEqual(0, failures.Length, string.Join(Environment.NewLine, failures));
        Assert.AreEqual("scripted.library-only", results.Single(result => result.Skipped is not null).Id);
    }

    [TestMethod]
    public async Task RunnerReportsAnExpectationThatDoesNotHold()
    {
        NurtClientOptions options = TestSupport.ProcessOptions(BackendKind.Process);
        var runner = new BindingCaseRunner(BackendKind.Process, () => options);

        BindingCaseRunner.CaseResult result = await runner.RunAsync(TestSupport.Json(MismatchCase));

        string failure = result.Failure ?? throw new AssertFailedException("the case passed");
        StringAssert.Contains(failure, "step 1 (run)");
        StringAssert.Contains(failure, "/text is \"hello\", expected \"other\"");
        StringAssert.Contains(failure, "/report is present");
    }

    [TestMethod]
    public async Task CasesPassOnTheProcessBackend()
    {
        string executable = TestSupport.Variable(TestSupport.ServeVariable)
            ?? Inconclusive<string>(TestSupport.ServeVariable + " does not name an executable that implements nurt serve --stdio");
        IReadOnlyList<string> arguments = TestSupport.Variable(TestSupport.ServeArgumentsVariable) is { } line
            ? line.Split(' ', StringSplitOptions.RemoveEmptyEntries)
            : NurtClientOptions.DefaultServeArguments;
        var options = new NurtClientOptions { Backend = BackendKind.Process, ExecutablePath = executable, ServeArguments = arguments };

        await RunRepositoryCasesAsync(new BindingCaseRunner(BackendKind.Process, () => options));
    }

    [TestMethod]
    public async Task CasesPassOnTheLibraryBackend()
    {
        string library = TestSupport.Variable(TestSupport.LibraryVariable)
            ?? Inconclusive<string>(TestSupport.LibraryVariable + " does not name a build of libnurt");
        var options = new NurtClientOptions { Backend = BackendKind.Library, LibraryPath = library };

        await RunRepositoryCasesAsync(new BindingCaseRunner(BackendKind.Library, () => options));
    }

    private static string Describe(BindingCaseRunner.CaseResult result) => result.Id + ": " + result.Failure;

    private static T Inconclusive<T>(string message) => throw new AssertInconclusiveException(message);

    private static async Task<List<BindingCaseRunner.CaseResult>> RunAllAsync(BindingCaseRunner runner, List<JsonElement> cases)
    {
        var results = new List<BindingCaseRunner.CaseResult>();
        foreach (JsonElement testCase in cases)
        {
            results.Add(await runner.RunAsync(testCase));
        }

        return results;
    }

    private async Task RunRepositoryCasesAsync(BindingCaseRunner runner)
    {
        List<JsonElement> cases = BindingCaseRunner.LoadCases(TestSupport.RequireRepositoryFile("conformance", "bindings"));
        List<BindingCaseRunner.CaseResult> results = await RunAllAsync(runner, cases);

        foreach (BindingCaseRunner.CaseResult skipped in results.Where(result => result.Skipped is not null))
        {
            TestContext.WriteLine("skipped " + skipped.Id + ": " + skipped.Skipped);
        }

        foreach (BindingCaseRunner.CaseResult optional in results.Where(result => !result.Mandatory && result.Failure is not null))
        {
            TestContext.WriteLine("optional case failed " + Describe(optional));
        }

        string[] failures = results.Where(result => result.Mandatory && result.Failure is not null).Select(Describe).ToArray();
        Assert.AreEqual(0, failures.Length, string.Join(Environment.NewLine, failures));
    }
}
