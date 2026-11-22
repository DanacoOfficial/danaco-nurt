using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;

namespace Danaco.Nurt.Tests;

/// <summary>
/// Runs binding cases <c>nurt-binding-cases/1</c> against a backend of <see cref="Client"/>.
/// Every case gets a new client and a new temporary directory; the calls of the cases are
/// mapped to the methods of <see cref="Client"/> and <see cref="Execution"/>, and the results
/// to the result views defined by the format.
/// </summary>
/// <remarks>
/// Two invalid calls are reported by the platform exceptions of .NET instead of
/// <see cref="CallException"/>: a missing argument raises <see cref="ArgumentException"/> and a
/// call on a disposed client raises <see cref="ObjectDisposedException"/>. Both correspond to
/// <c>NURT_E_ARG</c> (a null or invalid argument) and are reported as call errors with that status.
/// </remarks>
internal sealed partial class BindingCaseRunner
{
    /// <summary>Name and version of the case format.</summary>
    public const string Format = "nurt-binding-cases/1";

    /// <summary>Variable through which the fake server of the conformance suite answers one case only.</summary>
    public const string CaseVariable = "NURT_CONFORMANCE_CASE";

    private const string InvalidArgument = "NURT_E_ARG";
    private const string OptionalPrefix = "optional:";
    private const string FeaturePrefix = "feature:";
    private const string OutcomeOk = "ok";
    private const string OutcomeLanguageError = "language_error";
    private const string OutcomeCallError = "call_error";

    private static readonly TimeSpan EventLimit = TimeSpan.FromSeconds(30);
    private static readonly UTF8Encoding Utf8NoMark = new(encoderShouldEmitUTF8Identifier: false);

    private static readonly string[] RequestArguments = { "program", "ast", "cursor", "accept", "variant", "options", "raise_on_error" };

    private static readonly Dictionary<string, HashSet<string>> CallArguments = new(StringComparer.Ordinal)
    {
        ["run"] = Names(RequestArguments),
        ["start"] = Names(RequestArguments),
        ["check"] = Names("program", "ast", "variant", "options"),
        ["parse"] = Names("program", "variant"),
        ["print"] = Names("ast", "variant"),
        ["correct"] = Names("program", "variant"),
        ["page"] = Names("cursor", "limit"),
        ["describe"] = Names("layer", "name", "language"),
        ["help"] = Names("topic"),
        ["catalog"] = Names("what"),
        ["schema"] = Names("which"),
        ["history"] = Names("query"),
        ["space"] = Names("volume"),
        ["info"] = Names(),
        ["close"] = Names(),
        ["wait"] = Names("timeout_ms"),
        ["cancel"] = Names(),
        ["events"] = Names(),
    };

    private static readonly HashSet<string> ExecutionCalls = Names("wait", "cancel", "events");

    private static readonly HashSet<string> OptionNames = Names(
        "preview", "base", "limit", "fields", "format", "verbosity", "diff", "diff_context",
        "max_chars", "save_to", "timeout", "session", "language", "script");

    private static readonly HashSet<string> ExpectMembers = Names("outcome", "status", "code", "fields", "present", "absent");

    private static readonly HashSet<string> Outcomes = Names(OutcomeOk, OutcomeLanguageError, OutcomeCallError);

    private readonly BackendKind _backend;
    private readonly Func<ClientOptions> _options;
    private HashSet<string>? _features;

    /// <summary>Creates a runner for one backend.</summary>
    /// <param name="backend"><see cref="BackendKind.Process"/> or <see cref="BackendKind.Library"/>.</param>
    /// <param name="options">Creates the options of the client of each case.</param>
    public BindingCaseRunner(BackendKind backend, Func<ClientOptions> options)
    {
        _backend = backend;
        _options = options;
    }

    private string BackendName => _backend == BackendKind.Library ? "library" : "process";

    /// <summary>Reads the cases of every file <c>*.json</c> of the format <see cref="Format"/> in a directory, in the order of the file names.</summary>
    public static List<JsonElement> LoadCases(string directory)
    {
        var cases = new List<JsonElement>();
        foreach (string file in Directory.GetFiles(directory, "*.json").OrderBy(name => name, StringComparer.Ordinal))
        {
            cases.AddRange(CasesOf(TestSupport.Json(File.ReadAllText(file))));
        }

        return cases;
    }

    /// <summary>Returns the cases of a document of the format <see cref="Format"/>; none for a document of another format.</summary>
    public static List<JsonElement> CasesOf(JsonElement document)
    {
        if (document.ValueKind != JsonValueKind.Object
            || Text(document, "format") != Format
            || !document.TryGetProperty("cases", out JsonElement cases))
        {
            return new List<JsonElement>();
        }

        return cases.EnumerateArray().ToList();
    }

    /// <summary>Lists the calls, arguments, options and expectations of a case that the runner does not support.</summary>
    public static List<string> Validate(JsonElement testCase)
    {
        var problems = new List<string>();
        string id = Text(testCase, "id") ?? "<no id>";
        if (!testCase.TryGetProperty("steps", out JsonElement steps) || steps.ValueKind != JsonValueKind.Array || steps.GetArrayLength() == 0)
        {
            problems.Add(id + ": the case has no steps");
            return problems;
        }

        int number = 0;
        foreach (JsonElement step in steps.EnumerateArray())
        {
            number++;
            string where = id + " step " + number.ToString(CultureInfo.InvariantCulture);
            string? call = Text(step, "call");
            if (call is null || !CallArguments.TryGetValue(call, out HashSet<string>? allowed))
            {
                problems.Add(where + ": unsupported call " + (call ?? "<none>"));
                continue;
            }

            if (step.TryGetProperty("args", out JsonElement arguments))
            {
                foreach (JsonProperty argument in arguments.EnumerateObject())
                {
                    if (!allowed.Contains(argument.Name))
                    {
                        problems.Add(where + ": unsupported argument " + argument.Name + " of " + call);
                    }
                }

                if (arguments.TryGetProperty("options", out JsonElement options))
                {
                    foreach (JsonProperty option in options.EnumerateObject())
                    {
                        if (!OptionNames.Contains(option.Name))
                        {
                            problems.Add(where + ": unsupported option " + option.Name);
                        }
                    }
                }
            }

            if (ExecutionCalls.Contains(call) && (Integer(step, "execution") is not { } target || target < 1 || target >= number))
            {
                problems.Add(where + ": execution must name an earlier step");
            }

            if (!step.TryGetProperty("expect", out JsonElement expect) || expect.ValueKind != JsonValueKind.Object)
            {
                problems.Add(where + ": the step has no expectation");
                continue;
            }

            foreach (JsonProperty member in expect.EnumerateObject())
            {
                if (!ExpectMembers.Contains(member.Name))
                {
                    problems.Add(where + ": unsupported expectation " + member.Name);
                }
            }

            if (Text(expect, "outcome") is not { } outcome || !Outcomes.Contains(outcome))
            {
                problems.Add(where + ": unknown outcome");
            }
        }

        return problems;
    }

    /// <summary>Runs one case; failures of the case data and of the calls are reported in the result.</summary>
    public async Task<CaseResult> RunAsync(JsonElement testCase)
    {
        string id = Text(testCase, "id") ?? "<no id>";
        bool mandatory = !(Text(testCase, "class") ?? "mandatory").StartsWith(OptionalPrefix, StringComparison.Ordinal);
        try
        {
            return await RunCaseAsync(testCase, id, mandatory);
        }
        catch (InvalidDataException exception)
        {
            return CaseResult.Fail(id, mandatory, exception.Message);
        }
        catch (InvalidOperationException exception)
        {
            return CaseResult.Fail(id, mandatory, exception.Message);
        }
        catch (FormatException exception)
        {
            return CaseResult.Fail(id, mandatory, exception.Message);
        }
        catch (KeyNotFoundException exception)
        {
            return CaseResult.Fail(id, mandatory, exception.Message);
        }
        catch (OperationCanceledException)
        {
            return CaseResult.Fail(id, mandatory, "the case did not finish in time");
        }
        catch (CallException exception)
        {
            return CaseResult.Fail(id, mandatory, exception.StatusName + ": " + exception.Message);
        }
    }

    private static HashSet<string> Names(params string[] names) => new(names, StringComparer.Ordinal);

    private static string? Text(JsonElement element, string name) =>
        element.ValueKind == JsonValueKind.Object && element.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;

    private static int? Integer(JsonElement element, string name) =>
        element.ValueKind == JsonValueKind.Object && element.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.Number
            ? value.GetInt32()
            : null;

    private static string? Text(JsonObject arguments, string name) => arguments[name] switch
    {
        null => null,
        JsonValue value when value.GetValueKind() == JsonValueKind.String => value.GetValue<string>(),
        _ => throw new InvalidDataException("argument " + name + " must be a text"),
    };

    private static int? Integer(JsonObject arguments, string name) => arguments[name] switch
    {
        null => null,
        JsonValue value when value.GetValueKind() == JsonValueKind.Number => value.GetValue<int>(),
        _ => throw new InvalidDataException("argument " + name + " must be an integer"),
    };

    private static bool? Boolean(JsonObject arguments, string name) => arguments[name] switch
    {
        null => null,
        JsonValue value when value.GetValueKind() is JsonValueKind.True or JsonValueKind.False => value.GetValue<bool>(),
        _ => throw new InvalidDataException("argument " + name + " must be a boolean"),
    };

    private static JsonNode? Node(JsonElement element) => JsonNode.Parse(element.GetRawText());

    private static JsonNode? Member(JsonElement element, string name) =>
        element.TryGetProperty(name, out JsonElement value) ? Node(value) : null;

    private static JsonElement? OptionalElement(JsonNode? node) =>
        node is null ? null : TestSupport.Json(node.ToJsonString());

    private static JsonElement RequiredElement(JsonNode? node, string name) =>
        node is null ? throw new ArgumentNullException(name) : TestSupport.Json(node.ToJsonString());

    private static void WriteEntries(JsonElement testCase, string directory)
    {
        if (!testCase.TryGetProperty("fs", out JsonElement entries))
        {
            return;
        }

        foreach (JsonElement entry in entries.EnumerateArray())
        {
            string relative = Text(entry, "path") ?? throw new InvalidDataException("an entry of fs has no path");
            string path = Path.Combine(directory, relative.Replace('/', Path.DirectorySeparatorChar));
            switch (Text(entry, "type"))
            {
                case "dir":
                    Directory.CreateDirectory(path);
                    break;
                case "file":
                    Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                    File.WriteAllText(path, Text(entry, "content_utf8") ?? string.Empty, Utf8NoMark);
                    break;
                default:
                    throw new InvalidDataException("the entry " + relative + " of fs has an unsupported type");
            }
        }
    }

    private static async Task<StepResult> InvokeAsync(CaseState state, string call, JsonObject arguments, int? execution, int number)
    {
        try
        {
            return await DispatchAsync(state, call, arguments, execution, number);
        }
        catch (LanguageException exception)
        {
            return StepResult.Language(exception.Error?.Code, exception.Response is { } response ? ResponseView(response) : new JsonObject());
        }
        catch (CallException exception)
        {
            return StepResult.CallFailure(exception.StatusName, exception.Message);
        }
        catch (ObjectDisposedException exception)
        {
            return StepResult.CallFailure(InvalidArgument, exception.Message);
        }
        catch (ArgumentException exception)
        {
            return StepResult.CallFailure(InvalidArgument, exception.Message);
        }
    }

    private static async Task<StepResult> DispatchAsync(CaseState state, string call, JsonObject arguments, int? execution, int number)
    {
        Client client = state.Client;
        switch (call)
        {
            case "run":
                return FromResponse(await client.RunAsync(RequestOf(arguments)!, OptionsOf(arguments)));
            case "start":
                state.Executions[number] = await client.StartAsync(RequestOf(arguments)!, OptionsOf(arguments));
                return StepResult.Success(new JsonObject());
            case "wait":
                Execution waited = state.ExecutionOf(execution);
                Response response = Integer(arguments, "timeout_ms") is { } milliseconds
                    ? await waited.WaitAsync(TimeSpan.FromMilliseconds(milliseconds))
                    : await waited.WaitAsync();
                return FromResponse(response);
            case "cancel":
                await state.ExecutionOf(execution).CancelAsync();
                return StepResult.Success(new JsonObject());
            case "events":
                return StepResult.Success(new JsonObject { ["value"] = await EventsAsync(state.ExecutionOf(execution)) });
            case "check":
                return CheckView(await client.CheckAsync(RequestOf(arguments)!, OptionsOf(arguments)));
            case "parse":
                return ParseView(await client.ParseAsync(Text(arguments, "program")!, VariantOf(arguments)));
            case "print":
                return TextResult(await client.PrintAsync(RequiredElement(arguments["ast"], "ast"), VariantOf(arguments)));
            case "correct":
                return ValueResult(Node((await client.CorrectAsync(Text(arguments, "program")!, VariantOf(arguments))).Result));
            case "page":
                return FromResponse(await client.PageAsync(Text(arguments, "cursor")!, Integer(arguments, "limit")));
            case "describe":
                return TextResult(await client.DescribeAsync(Text(arguments, "layer") ?? "core", Text(arguments, "name"), Text(arguments, "language")));
            case "help":
                return TextResult(await client.HelpAsync(Text(arguments, "topic")));
            case "catalog":
                return ValueResult(Node(await client.CatalogAsync(OptionalElement(arguments["what"]))));
            case "schema":
                return ValueResult(Node(await client.SchemaAsync(Text(arguments, "which")!)));
            case "history":
                return ValueResult(Node(await client.HistoryAsync(OptionalElement(arguments["query"]))));
            case "space":
                return ValueResult(Node(await client.SpaceAsync(Text(arguments, "volume"))));
            case "info":
                return ValueResult(Node(await client.InfoAsync()));
            case "close":
                await client.DisposeAsync();
                return StepResult.Success(new JsonObject());
            default:
                throw new InvalidDataException("unsupported call " + call);
        }
    }

    private static Request? RequestOf(JsonObject arguments)
    {
        if (Text(arguments, "program") is { } program)
        {
            return Request.FromProgram(program, VariantOrNull(arguments));
        }

        if (arguments["ast"] is { } ast)
        {
            return Request.FromAst(TestSupport.Json(ast.ToJsonString()));
        }

        if (Text(arguments, "cursor") is { } cursor)
        {
            return Request.FromCursor(cursor);
        }

        return Text(arguments, "accept") is { } accept ? Request.FromAccept(accept) : null;
    }

    private static RunOptions? OptionsOf(JsonObject arguments)
    {
        bool raise = Boolean(arguments, "raise_on_error") ?? false;
        JsonObject? options = arguments["options"] as JsonObject;
        if (options is null)
        {
            return raise ? new RunOptions { ThrowOnLanguageError = true } : null;
        }

        foreach (KeyValuePair<string, JsonNode?> option in options)
        {
            if (!OptionNames.Contains(option.Key))
            {
                throw new InvalidDataException("unsupported option " + option.Key);
            }
        }

        return new RunOptions
        {
            Preview = Boolean(options, "preview"),
            Base = Text(options, "base"),
            Limit = Integer(options, "limit"),
            Fields = options["fields"] is JsonArray fields ? fields.Select(item => item!.GetValue<string>()).ToList() : null,
            Format = Text(options, "format") is { } format ? FormatOf(format) : null,
            Verbosity = Text(options, "verbosity") is { } verbosity ? VerbosityOf(verbosity) : null,
            Diff = Text(options, "diff") is { } diff ? DiffOf(diff) : null,
            DiffContext = Integer(options, "diff_context"),
            MaxChars = Integer(options, "max_chars"),
            SaveTo = Text(options, "save_to"),
            Timeout = Text(options, "timeout") is { } timeout ? DurationOf(timeout) : null,
            Session = Text(options, "session"),
            Language = Text(options, "language"),
            Script = Boolean(options, "script"),
            ThrowOnLanguageError = raise,
        };
    }

    private static NotationVariant? VariantOrNull(JsonObject arguments) => Text(arguments, "variant") switch
    {
        null => null,
        "a" => NotationVariant.A,
        "b" => NotationVariant.B,
        "c" => NotationVariant.C,
        var other => throw new InvalidDataException("unknown variant " + other),
    };

    private static NotationVariant VariantOf(JsonObject arguments) => VariantOrNull(arguments) ?? NotationVariant.A;

    private static OutputFormat FormatOf(string value) => value switch
    {
        "text" => OutputFormat.Text,
        "json" => OutputFormat.Json,
        "markdown" => OutputFormat.Markdown,
        "compact" => OutputFormat.Compact,
        _ => throw new InvalidDataException("unknown format " + value),
    };

    private static Verbosity VerbosityOf(string value) => value switch
    {
        "quiet" => Verbosity.Quiet,
        "normal" => Verbosity.Normal,
        "detailed" => Verbosity.Detailed,
        _ => throw new InvalidDataException("unknown verbosity " + value),
    };

    private static DiffMode DiffOf(string value) => value switch
    {
        "none" => DiffMode.None,
        "stat" => DiffMode.Stat,
        "unified" => DiffMode.Unified,
        _ => throw new InvalidDataException("unknown diff mode " + value),
    };

    private static TimeSpan DurationOf(string text)
    {
        Match match = DurationPattern().Match(text);
        if (!match.Success)
        {
            throw new InvalidDataException("invalid duration " + text);
        }

        CaptureCollection amounts = match.Groups["amount"].Captures;
        CaptureCollection units = match.Groups["unit"].Captures;
        TimeSpan total = TimeSpan.Zero;
        for (int index = 0; index < amounts.Count; index++)
        {
            long amount = long.Parse(amounts[index].ValueSpan, NumberStyles.None, CultureInfo.InvariantCulture);
            total += units[index].Value switch
            {
                "ms" => TimeSpan.FromMilliseconds(amount),
                "s" => TimeSpan.FromSeconds(amount),
                "m" => TimeSpan.FromMinutes(amount),
                "h" => TimeSpan.FromHours(amount),
                _ => TimeSpan.FromDays(amount),
            };
        }

        return total;
    }

    private static async Task<JsonArray> EventsAsync(Execution execution)
    {
        using var limit = new CancellationTokenSource(EventLimit);
        var events = new JsonArray();
        await foreach (ExecutionEvent item in execution.ReadEventsAsync(limit.Token))
        {
            events.Add(Node(item.Element));
        }

        return events;
    }

    private static JsonObject ResponseView(Response response)
    {
        var view = new JsonObject
        {
            ["ok"] = response.Ok,
            ["report"] = Node(response.Report.Element),
        };
        if (response.Text is { } text)
        {
            view["text"] = text;
        }

        if (response.Error is { } error)
        {
            view["error"] = Node(error.Element);
        }

        if (response.NextCursor is { } cursor)
        {
            view["next_cursor"] = cursor;
        }

        return view;
    }

    private static StepResult FromResponse(Response response) => response.Error is { } error
        ? StepResult.Language(error.Code, ResponseView(response))
        : StepResult.Success(ResponseView(response));

    private static StepResult TextResult(string text) => StepResult.Success(new JsonObject { ["value"] = text });

    /// <summary>Result view <c>{"value": object}</c>; an object with a member <c>error</c> is a language error.</summary>
    private static StepResult ValueResult(JsonNode? value)
    {
        var view = new JsonObject { ["value"] = value };
        return value is JsonObject result && result["error"] is JsonObject error && error["code"] is JsonValue code
            ? StepResult.Language(code.GetValue<string>(), view)
            : StepResult.Success(view);
    }

    private static StepResult CheckView(CheckResult check)
    {
        JsonNode? value = Node(check.Result);
        if (check.Error is { } error && value is JsonObject result && !result.ContainsKey("error"))
        {
            result["error"] = Node(error.Element);
        }

        return ValueResult(value);
    }

    private static StepResult ParseView(ParseResult parse)
    {
        var value = new JsonObject();
        if (parse.Ast is { } ast)
        {
            value["ast"] = Node(ast);
        }

        if (parse.Diagnostics is { } diagnostics)
        {
            value["diagnostics"] = Node(diagnostics);
        }

        if (parse.Error is { } error)
        {
            value["error"] = Node(error.Element);
        }

        return ValueResult(value);
    }

    private static string? Compare(StepResult result, JsonObject expect)
    {
        var problems = new List<string>();
        string outcome = Text(expect, "outcome") ?? OutcomeOk;
        if (outcome != result.Outcome)
        {
            problems.Add("outcome " + result.Outcome + (result.Detail is null ? string.Empty : " (" + result.Detail + ")") + ", expected " + outcome);
        }

        if (Text(expect, "status") is { } status && status != result.Status)
        {
            problems.Add("status " + (result.Status ?? "none") + ", expected " + status);
        }

        if (Text(expect, "code") is { } code && code != result.Code)
        {
            problems.Add("code " + (result.Code ?? "none") + ", expected " + code);
        }

        if (expect["fields"] is JsonObject fields)
        {
            foreach (KeyValuePair<string, JsonNode?> field in fields)
            {
                if (!TryResolve(result.View, field.Key, out JsonNode? actual))
                {
                    problems.Add(field.Key + " is missing");
                }
                else if (!JsonEquals(actual, field.Value))
                {
                    problems.Add(field.Key + " is " + Show(actual) + ", expected " + Show(field.Value));
                }
            }
        }

        foreach (string pointer in Pointers(expect, "present"))
        {
            if (!TryResolve(result.View, pointer, out _))
            {
                problems.Add(pointer + " is missing");
            }
        }

        foreach (string pointer in Pointers(expect, "absent"))
        {
            if (TryResolve(result.View, pointer, out _))
            {
                problems.Add(pointer + " is present");
            }
        }

        return problems.Count == 0 ? null : string.Join("; ", problems);
    }

    private static List<string> Pointers(JsonObject expect, string name) =>
        expect[name] is JsonArray pointers ? pointers.Select(item => item!.GetValue<string>()).ToList() : new List<string>();

    private static string Show(JsonNode? node) => node?.ToJsonString() ?? "null";

    /// <summary>Resolves a JSON pointer (RFC 6901); a member whose value is <c>null</c> exists.</summary>
    private static bool TryResolve(JsonNode? root, string pointer, out JsonNode? value)
    {
        value = root;
        if (pointer.Length == 0)
        {
            return true;
        }

        if (pointer[0] != '/')
        {
            value = null;
            return false;
        }

        foreach (string segment in pointer[1..].Split('/'))
        {
            string token = segment.Replace("~1", "/", StringComparison.Ordinal).Replace("~0", "~", StringComparison.Ordinal);
            switch (value)
            {
                case JsonObject members when members.TryGetPropertyValue(token, out JsonNode? next):
                    value = next;
                    break;
                case JsonArray items when int.TryParse(token, NumberStyles.None, CultureInfo.InvariantCulture, out int index) && index < items.Count:
                    value = items[index];
                    break;
                default:
                    value = null;
                    return false;
            }
        }

        return true;
    }

    /// <summary>Compares two JSON values; numbers are equal when their values are equal.</summary>
    private static bool JsonEquals(JsonNode? left, JsonNode? right)
    {
        switch (left)
        {
            case null:
                return right is null;
            case JsonObject leftMembers:
                if (right is not JsonObject rightMembers || leftMembers.Count != rightMembers.Count)
                {
                    return false;
                }

                foreach (KeyValuePair<string, JsonNode?> member in leftMembers)
                {
                    if (!rightMembers.TryGetPropertyValue(member.Key, out JsonNode? other) || !JsonEquals(member.Value, other))
                    {
                        return false;
                    }
                }

                return true;
            case JsonArray leftItems:
                if (right is not JsonArray rightItems || leftItems.Count != rightItems.Count)
                {
                    return false;
                }

                for (int index = 0; index < leftItems.Count; index++)
                {
                    if (!JsonEquals(leftItems[index], rightItems[index]))
                    {
                        return false;
                    }
                }

                return true;
            default:
                if (right is not JsonValue || left.GetValueKind() != right.GetValueKind())
                {
                    return false;
                }

                return left.GetValueKind() switch
                {
                    JsonValueKind.Number => NumberOf(left) == NumberOf(right),
                    JsonValueKind.String => left.GetValue<string>() == right.GetValue<string>(),
                    _ => true,
                };
        }
    }

    private static double NumberOf(JsonNode node) =>
        double.Parse(node.ToJsonString(), NumberStyles.Float, CultureInfo.InvariantCulture);

    /// <summary>
    /// Replaces the placeholders <c>${base}</c> and <c>${stepN/pointer}</c>. A text that consists
    /// of one placeholder is replaced by the JSON value itself; inside a longer text the value is
    /// inserted as text.
    /// </summary>
    private static JsonNode? Substitute(JsonNode? node, CaseState state)
    {
        switch (node)
        {
            case JsonObject members:
                var replaced = new JsonObject();
                foreach (KeyValuePair<string, JsonNode?> member in members)
                {
                    replaced[member.Key] = Substitute(member.Value, state);
                }

                return replaced;
            case JsonArray items:
                var array = new JsonArray();
                foreach (JsonNode? item in items)
                {
                    array.Add(Substitute(item, state));
                }

                return array;
            case JsonValue text when text.GetValueKind() == JsonValueKind.String:
                string raw = text.GetValue<string>();
                Match whole = PlaceholderPattern().Match(raw);
                if (whole.Success && whole.Index == 0 && whole.Length == raw.Length)
                {
                    return Resolve(whole, state);
                }

                return JsonValue.Create(PlaceholderPattern().Replace(raw, match => TextOf(Resolve(match, state))));
            default:
                return node?.DeepClone();
        }
    }

    private static JsonNode? Resolve(Match match, CaseState state)
    {
        if (match.Groups["base"].Success)
        {
            return JsonValue.Create(state.Base);
        }

        int step = int.Parse(match.Groups["step"].ValueSpan, NumberStyles.None, CultureInfo.InvariantCulture);
        if (step < 1 || step > state.Views.Count)
        {
            throw new InvalidDataException("the placeholder " + match.Value + " names a step that has not run");
        }

        if (!TryResolve(state.Views[step - 1], match.Groups["pointer"].Value, out JsonNode? value))
        {
            throw new InvalidDataException("the placeholder " + match.Value + " names no value");
        }

        return value?.DeepClone();
    }

    private static string TextOf(JsonNode? node) =>
        node is JsonValue value && value.GetValueKind() == JsonValueKind.String ? value.GetValue<string>() : Show(node);

    [GeneratedRegex(@"\$\{(?:(?<base>base)|step(?<step>[0-9]+)(?<pointer>/[^}]*)?)\}", RegexOptions.CultureInvariant)]
    private static partial Regex PlaceholderPattern();

    [GeneratedRegex("^(?:(?<amount>[0-9]+)(?<unit>ms|s|m|h|d))+$", RegexOptions.CultureInvariant)]
    private static partial Regex DurationPattern();

    private async Task<CaseResult> RunCaseAsync(JsonElement testCase, string id, bool mandatory)
    {
        if (await SkipReasonAsync(testCase) is { } reason)
        {
            return CaseResult.Skip(id, mandatory, reason);
        }

        List<string> problems = Validate(testCase);
        if (problems.Count > 0)
        {
            return CaseResult.Fail(id, mandatory, string.Join("; ", problems));
        }

        string directory = Path.Combine(Path.GetTempPath(), "nurt-binding-" + Guid.NewGuid().ToString("N", CultureInfo.InvariantCulture));
        Directory.CreateDirectory(directory);
        string? previous = Environment.GetEnvironmentVariable(CaseVariable);
        try
        {
            WriteEntries(testCase, directory);
            if (_backend == BackendKind.Process)
            {
                Environment.SetEnvironmentVariable(CaseVariable, id);
            }

            string? failure = await RunStepsAsync(testCase, directory.Replace('\\', '/'));
            return failure is null ? CaseResult.Pass(id, mandatory) : CaseResult.Fail(id, mandatory, failure);
        }
        finally
        {
            Environment.SetEnvironmentVariable(CaseVariable, previous);
            Directory.Delete(directory, recursive: true);
        }
    }

    private async Task<string?> RunStepsAsync(JsonElement testCase, string basePath)
    {
        var state = new CaseState(new Client(_options()), basePath);
        try
        {
            int number = 0;
            foreach (JsonElement step in testCase.GetProperty("steps").EnumerateArray())
            {
                number++;
                string call = Text(step, "call")!;
                JsonObject arguments = Substitute(Member(step, "args"), state) as JsonObject ?? new JsonObject();
                StepResult result = await InvokeAsync(state, call, arguments, Integer(step, "execution"), number);
                state.Views.Add(result.View);
                JsonObject expect = Substitute(Member(step, "expect"), state) as JsonObject ?? new JsonObject();
                if (Compare(result, expect) is { } mismatch)
                {
                    return "step " + number.ToString(CultureInfo.InvariantCulture) + " (" + call + "): " + mismatch;
                }
            }

            return null;
        }
        finally
        {
            await state.CloseAsync();
        }
    }

    private async Task<string?> SkipReasonAsync(JsonElement testCase)
    {
        if (testCase.TryGetProperty("backends", out JsonElement backends)
            && !backends.EnumerateArray().Any(item => item.ValueKind == JsonValueKind.String && item.GetString() == BackendName))
        {
            return "not run on the " + BackendName + " backend";
        }

        var features = new List<string>();
        if (Text(testCase, "class") is { } kind && kind.StartsWith(OptionalPrefix, StringComparison.Ordinal))
        {
            features.Add(kind[OptionalPrefix.Length..]);
        }

        if (testCase.TryGetProperty("requires", out JsonElement requires))
        {
            foreach (JsonElement item in requires.EnumerateArray())
            {
                string condition = item.GetString() ?? string.Empty;
                if (condition.StartsWith(FeaturePrefix, StringComparison.Ordinal))
                {
                    features.Add(condition[FeaturePrefix.Length..]);
                    continue;
                }

                bool holds = condition switch
                {
                    "os:linux" => OperatingSystem.IsLinux(),
                    "os:macos" => OperatingSystem.IsMacOS(),
                    "os:windows" => OperatingSystem.IsWindows(),
                    "os:unix" => !OperatingSystem.IsWindows(),
                    _ => false,
                };
                if (!holds)
                {
                    return "the condition " + condition + " does not hold";
                }
            }
        }

        if (features.Count > 0)
        {
            HashSet<string> offered = await FeaturesAsync();
            foreach (string feature in features)
            {
                if (!offered.Contains(feature))
                {
                    return "the feature " + feature + " is not offered";
                }
            }
        }

        return null;
    }

    private async Task<HashSet<string>> FeaturesAsync()
    {
        if (_features is null)
        {
            var features = Names();
            var client = new Client(_options());
            try
            {
                JsonElement info = await client.InfoAsync();
                if (info.ValueKind == JsonValueKind.Object
                    && info.TryGetProperty("features", out JsonElement list)
                    && list.ValueKind == JsonValueKind.Array)
                {
                    foreach (JsonElement item in list.EnumerateArray())
                    {
                        if (item.ValueKind == JsonValueKind.String)
                        {
                            features.Add(item.GetString()!);
                        }
                    }
                }
            }
            finally
            {
                await client.DisposeAsync();
            }

            _features = features;
        }

        return _features;
    }

    /// <summary>Outcome of one case.</summary>
    internal sealed class CaseResult
    {
        private CaseResult(string id, bool mandatory, string? skipped, string? failure)
        {
            Id = id;
            Mandatory = mandatory;
            Skipped = skipped;
            Failure = failure;
        }

        public string Id { get; }

        public bool Mandatory { get; }

        /// <summary>Reason the case was not run; <see langword="null"/> when it ran.</summary>
        public string? Skipped { get; }

        /// <summary>Description of the first failing step; <see langword="null"/> when the case passed or was skipped.</summary>
        public string? Failure { get; }

        public static CaseResult Pass(string id, bool mandatory) => new(id, mandatory, null, null);

        public static CaseResult Skip(string id, bool mandatory, string reason) => new(id, mandatory, reason, null);

        public static CaseResult Fail(string id, bool mandatory, string failure) => new(id, mandatory, null, failure);
    }

    /// <summary>Outcome and result view of one step.</summary>
    private sealed class StepResult
    {
        private StepResult(string outcome, string? status, string? code, string? detail, JsonNode view)
        {
            Outcome = outcome;
            Status = status;
            Code = code;
            Detail = detail;
            View = view;
        }

        public string Outcome { get; }

        public string? Status { get; }

        public string? Code { get; }

        public string? Detail { get; }

        public JsonNode View { get; }

        public static StepResult Success(JsonNode view) => new(OutcomeOk, null, null, null, view);

        public static StepResult Language(string? code, JsonNode view) => new(OutcomeLanguageError, null, code, null, view);

        public static StepResult CallFailure(string status, string detail) => new(OutcomeCallError, status, null, detail, new JsonObject());
    }

    /// <summary>Client, temporary directory, result views and executions of a running case.</summary>
    private sealed class CaseState
    {
        public CaseState(Client client, string basePath)
        {
            Client = client;
            Base = basePath;
        }

        public Client Client { get; }

        /// <summary>Temporary directory of the case with <c>/</c> as separator; the value of <c>${base}</c>.</summary>
        public string Base { get; }

        public List<JsonNode?> Views { get; } = new();

        public Dictionary<int, Execution> Executions { get; } = new();

        public Execution ExecutionOf(int? step) =>
            step is { } number && Executions.TryGetValue(number, out Execution? execution)
                ? execution
                : throw new InvalidDataException("no execution was started by step " + (step?.ToString(CultureInfo.InvariantCulture) ?? "<none>"));

        public async Task CloseAsync()
        {
            foreach (Execution execution in Executions.Values)
            {
                await execution.DisposeAsync();
            }

            await Client.DisposeAsync();
        }
    }
}
