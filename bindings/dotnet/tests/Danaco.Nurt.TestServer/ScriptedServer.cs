// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/tests/Danaco.Nurt.TestServer/ScriptedServer.cs
// Responsibility:   Source file bindings/dotnet/tests/Danaco.Nurt.TestServer/ScriptedServer.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Text;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt.TestServer;

/// <summary>
/// JSON-RPC 2.0 server with one message per line. The first word of the program text of
/// <c>run</c> selects the behaviour:
/// <list type="bullet">
/// <item><c>echo TEXT</c>: report <c>ok</c> of kind <c>echo</c>, rendered text TEXT.</item>
/// <item><c>error</c>: report with the language error <c>E_SYNTAX</c>.</item>
/// <item><c>sleep</c>: waits for <c>cancel</c>, then reports <c>E_CANCELLED</c>.</item>
/// <item><c>delay MS</c>: answers after MS milliseconds.</item>
/// <item><c>events N</c>: sends N events <c>job.output</c> and <c>exec.finished</c>, then answers.</item>
/// <item><c>crash</c>: writes to standard error and exits with code 3.</item>
/// <item><c>garbage</c>: writes a line that is not JSON.</item>
/// <item><c>fail STATUS</c>: answers with the JSON-RPC error <c>-32000 - STATUS</c>.</item>
/// <item><c>page</c>: report with the cursor <c>c1</c>; the cursor <c>c1</c> returns the last page.</item>
/// </list>
/// The method <c>parse</c> of the text <c>error</c> returns the language error <c>E_LEXICAL</c>.
/// Every response to <c>run</c> and <c>page</c> also carries the received parameters in the
/// member <c>request</c>.
/// </summary>
internal sealed class ScriptedServer : IDisposable
{
    private const int CrashExitCode = 3;
    private const int StatusErrorBase = -32000;
    private const int MethodNotFound = -32601;

    private readonly ServerOptions _options;
    private readonly StreamReader _input;
    private readonly Stream _output;
    private readonly SemaphoreSlim _writeLock = new(1, 1);
    private readonly ConcurrentDictionary<long, TaskCompletionSource<bool>> _sleeping = new();
    private int _cancelled;
    private int _inflight;
    private int _maxInflightObserved;

    public ScriptedServer(ServerOptions options, Stream input, Stream output)
    {
        _options = options;
        _input = new StreamReader(input, new UTF8Encoding(false));
        _output = output;
    }

    public async Task<int> RunAsync()
    {
        var running = new List<Task>();
        while (await _input.ReadLineAsync().ConfigureAwait(false) is { } line)
        {
            if (line.Length == 0)
            {
                continue;
            }

            JsonObject message = JsonNode.Parse(line)!.AsObject();
            running.Add(Task.Run(() => HandleAsync(message)));
        }

        foreach (TaskCompletionSource<bool> sleeper in _sleeping.Values)
        {
            sleeper.TrySetResult(true);
        }

        await Task.WhenAll(running).ConfigureAwait(false);
        return 0;
    }

    public void Dispose()
    {
        _input.Dispose();
        _writeLock.Dispose();
    }

    private static JsonObject Report(string status, string kind) => new()
    {
        ["nurt"] = "1.0",
        ["status"] = status,
        ["kind"] = kind,
    };

    /// <summary>Result of <c>parse</c>: the language error <c>E_LEXICAL</c> for the text <c>error</c>, otherwise an empty tree.</summary>
    private static JsonObject ParseResultOf(string? program) => program == "error"
        ? new JsonObject
        {
            ["error"] = new JsonObject
            {
                ["code"] = "E_LEXICAL",
                ["message"] = "unterminated text",
                ["at"] = new JsonObject { ["line"] = 1, ["column"] = 1, ["stage"] = "1" },
            },
        }
        : new JsonObject
        {
            ["ast"] = new JsonObject { ["nurt"] = "1.0", ["program"] = new JsonArray() },
            ["diagnostics"] = new JsonArray(),
        };

    private static JsonObject ResponseOf(JsonObject report, string? text, JsonNode? request)
    {
        var response = new JsonObject { ["nurt"] = "1.0", ["report"] = report };
        if (text is not null)
        {
            response["rendered"] = new JsonObject { ["format"] = "text", ["text"] = text };
        }

        response["request"] = request?.DeepClone();
        return response;
    }

    private async Task HandleAsync(JsonObject message)
    {
        long id = message["id"]!.GetValue<long>();
        string method = message["method"]!.GetValue<string>();
        JsonNode? parameters = message["params"];
        switch (method)
        {
            case "initialize":
                var result = new JsonObject
                {
                    ["protocol"] = _options.Protocol,
                    ["version"] = "0.0.0-test",
                    ["abi"] = 1 << 16,
                    ["features"] = new JsonArray(),
                };
                if (_options.MaxInflight > 0)
                {
                    result["limits"] = new JsonObject { ["max_inflight"] = _options.MaxInflight, ["max_message_bytes"] = 1 << 24 };
                }

                await ResultAsync(id, result).ConfigureAwait(false);
                break;
            case "cancel":
                long target = parameters!["request_id"]!.GetValue<long>();
                Interlocked.Increment(ref _cancelled);
                bool running = _sleeping.TryRemove(target, out TaskCompletionSource<bool>? sleeper);
                if (running)
                {
                    sleeper!.TrySetResult(true);
                }

                await ResultAsync(id, new JsonObject { ["cancelled"] = running }).ConfigureAwait(false);
                break;
            case "run":
                await RunAsync(id, parameters!.AsObject()).ConfigureAwait(false);
                break;
            case "page":
                await PageAsync(id, parameters!["cursor"]!.GetValue<string>(), parameters).ConfigureAwait(false);
                break;
            case "check":
                await ResultAsync(id, new JsonObject
                {
                    ["bound"] = new JsonObject(),
                    ["effects"] = new JsonArray(),
                    ["diagnostics"] = new JsonArray(),
                }).ConfigureAwait(false);
                break;
            case "parse":
                await ResultAsync(id, ParseResultOf(parameters?["program"]?.GetValue<string>())).ConfigureAwait(false);
                break;
            case "print":
                await ResultAsync(id, new JsonObject { ["text"] = "echo printed" }).ConfigureAwait(false);
                break;
            case "correct":
                await ResultAsync(id, new JsonObject { ["program"] = "echo corrected", ["corrections"] = new JsonArray() }).ConfigureAwait(false);
                break;
            case "describe":
                await ResultAsync(id, new JsonObject { ["text"] = "description " + parameters!["subject"]!.GetValue<string>() + " " + parameters["language"]?.GetValue<string>() }).ConfigureAwait(false);
                break;
            case "help":
                await ResultAsync(id, JsonValue.Create("help " + (parameters?["topic"]?.GetValue<string>() ?? "index"))).ConfigureAwait(false);
                break;
            case "schema":
                string which = parameters!["name"]!.GetValue<string>();
                if (which == "unknown")
                {
                    await ErrorAsync(id, StatusErrorBase - 3, "unknown schema").ConfigureAwait(false);
                }
                else
                {
                    await ResultAsync(id, new JsonObject { ["title"] = which }).ConfigureAwait(false);
                }

                break;
            case "catalog":
                await ResultAsync(id, new JsonObject { ["groups"] = new JsonArray(), ["what"] = parameters?.DeepClone() }).ConfigureAwait(false);
                break;
            case "history":
                await ResultAsync(id, new JsonObject { ["entries"] = new JsonArray(), ["query"] = parameters?.DeepClone() }).ConfigureAwait(false);
                break;
            case "space":
                await ResultAsync(id, new JsonObject { ["volumes"] = new JsonArray(), ["volume"] = parameters?["volume"]?.DeepClone() }).ConfigureAwait(false);
                break;
            case "info":
                await ResultAsync(id, new JsonObject
                {
                    ["version"] = "0.0.0-test",
                    ["pid"] = Environment.ProcessId,
                    ["cancelled"] = Volatile.Read(ref _cancelled),
                    ["max_inflight_observed"] = Volatile.Read(ref _maxInflightObserved),
                }).ConfigureAwait(false);
                break;
            default:
                await ErrorAsync(id, MethodNotFound, "method not found: " + method).ConfigureAwait(false);
                break;
        }
    }

    private async Task RunAsync(long id, JsonObject envelope)
    {
        if (envelope["cursor"]?.GetValue<string>() is { } cursor)
        {
            await PageAsync(id, cursor, envelope).ConfigureAwait(false);
            return;
        }

        string program = envelope["program"]?.GetValue<string>() ?? "echo";
        int space = program.IndexOf(' ', StringComparison.Ordinal);
        string verb = space < 0 ? program : program[..space];
        string argument = space < 0 ? string.Empty : program[(space + 1)..];
        switch (verb)
        {
            case "echo":
                await ResultAsync(id, ResponseOf(Report("ok", "echo"), argument, envelope)).ConfigureAwait(false);
                break;
            case "error":
                JsonObject failed = Report("failed", "error");
                failed["error"] = new JsonObject
                {
                    ["code"] = "E_SYNTAX",
                    ["message"] = "unexpected token",
                    ["at"] = new JsonObject { ["line"] = 1, ["column"] = 1, ["length"] = 5, ["stage"] = "1" },
                    ["fix"] = new JsonArray("echo error"),
                    ["help"] = "syntax",
                    ["candidates"] = new JsonArray(new JsonObject { ["rank"] = 1, ["ref"] = "k1", ["program"] = "echo error" }),
                };
                await ResultAsync(id, ResponseOf(failed, "error", envelope)).ConfigureAwait(false);
                break;
            case "sleep":
                var wake = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
                _sleeping[id] = wake;
                await wake.Task.ConfigureAwait(false);
                JsonObject cancelled = Report("failed", "sleep");
                cancelled["error"] = new JsonObject { ["code"] = "E_CANCELLED", ["message"] = "cancelled" };
                await CancelledAsync(id, ResponseOf(cancelled, null, envelope)).ConfigureAwait(false);
                break;
            case "delay":
                int now = Interlocked.Increment(ref _inflight);
                UpdateMaximum(now);
                await Task.Delay(int.Parse(argument, CultureInfo.InvariantCulture)).ConfigureAwait(false);
                Interlocked.Decrement(ref _inflight);
                await ResultAsync(id, ResponseOf(Report("ok", "delay"), program, envelope)).ConfigureAwait(false);
                break;
            case "events":
                int count = int.Parse(argument, CultureInfo.InvariantCulture);
                for (int seq = 1; seq <= count; seq++)
                {
                    await EventAsync(id, "job.output", seq).ConfigureAwait(false);
                }

                await EventAsync(id, "exec.finished", count + 1).ConfigureAwait(false);
                await ResultAsync(id, ResponseOf(Report("ok", "events"), null, envelope)).ConfigureAwait(false);
                break;
            case "crash":
                await Console.Error.WriteLineAsync("fatal: test crash").ConfigureAwait(false);
                await Console.Error.FlushAsync().ConfigureAwait(false);
                Environment.Exit(CrashExitCode);
                break;
            case "garbage":
                await WriteLineAsync("not json").ConfigureAwait(false);
                break;
            case "fail":
                await ErrorAsync(id, StatusErrorBase - int.Parse(argument, CultureInfo.InvariantCulture), "forced failure").ConfigureAwait(false);
                break;
            case "page":
                JsonObject first = Report("ok", "page");
                first["next_cursor"] = "c1";
                await ResultAsync(id, ResponseOf(first, "page 1", envelope)).ConfigureAwait(false);
                break;
            default:
                await ResultAsync(id, ResponseOf(Report("ok", verb), program, envelope)).ConfigureAwait(false);
                break;
        }
    }

    private Task PageAsync(long id, string cursor, JsonNode? request)
    {
        JsonObject report = Report("ok", "page");
        if (cursor != "c1")
        {
            report["error"] = new JsonObject { ["code"] = "E_EXPIRED_CANDIDATE", ["message"] = "cursor expired" };
        }

        return ResultAsync(id, ResponseOf(report, "page 2", request));
    }

    private void UpdateMaximum(int value)
    {
        int current;
        do
        {
            current = Volatile.Read(ref _maxInflightObserved);
            if (value <= current)
            {
                return;
            }
        }
        while (Interlocked.CompareExchange(ref _maxInflightObserved, value, current) != current);
    }

    private Task EventAsync(long requestId, string kind, int seq) => WriteAsync(new JsonObject
    {
        ["jsonrpc"] = "2.0",
        ["method"] = "nurt/event",
        ["params"] = new JsonObject
        {
            ["t"] = kind,
            ["seq"] = seq,
            ["time"] = "2026-01-01T00:00:00Z",
            ["request"] = requestId.ToString(CultureInfo.InvariantCulture),
        },
    });

    private Task ResultAsync(long id, JsonNode? result) => WriteAsync(new JsonObject
    {
        ["jsonrpc"] = "2.0",
        ["id"] = id,
        ["result"] = result,
    });

    private Task ErrorAsync(long id, int code, string message) => WriteAsync(new JsonObject
    {
        ["jsonrpc"] = "2.0",
        ["id"] = id,
        ["error"] = new JsonObject { ["code"] = code, ["message"] = message, ["data"] = new JsonObject { ["source"] = "test" } },
    });

    /// <summary>Answer of a cancelled <c>run</c>: the error <c>NURT_E_CANCELLED</c> that carries the response.</summary>
    private Task CancelledAsync(long id, JsonObject response) => WriteAsync(new JsonObject
    {
        ["jsonrpc"] = "2.0",
        ["id"] = id,
        ["error"] = new JsonObject
        {
            ["code"] = StatusErrorBase - 2,
            ["message"] = "the request was cancelled",
            ["data"] = new JsonObject { ["status"] = "NURT_E_CANCELLED", ["response"] = response },
        },
    });

    private Task WriteAsync(JsonObject message) => WriteLineAsync(message.ToJsonString());

    private async Task WriteLineAsync(string line)
    {
        byte[] bytes = Encoding.UTF8.GetBytes(line + "\n");
        await _writeLock.WaitAsync().ConfigureAwait(false);
        try
        {
            await _output.WriteAsync(bytes).ConfigureAwait(false);
            await _output.FlushAsync().ConfigureAwait(false);
        }
        finally
        {
            _writeLock.Release();
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
