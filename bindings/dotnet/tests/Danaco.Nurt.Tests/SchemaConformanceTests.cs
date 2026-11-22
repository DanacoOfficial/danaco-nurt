using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

/// <summary>
/// The request envelope written by the binding and the members it reads from responses and
/// reports, checked against the published schemas <c>nurt-request/1</c> and <c>nurt-report/1</c>.
/// </summary>
[TestClass]
public sealed class SchemaConformanceTests
{
    private const string RequestSchema = "nurt-request-1.schema.json";
    private const string ReportSchema = "nurt-report-1.schema.json";

    /// <summary>Members of the report read by <see cref="Report"/>.</summary>
    private static readonly string[] ReportMembers = { "nurt", "status", "kind", "op", "next_cursor", "error" };

    /// <summary>Members of an error read by <see cref="LanguageError"/>.</summary>
    private static readonly string[] ErrorMembers = { "code", "message", "at", "fix", "help", "candidates" };

    /// <summary>Members of the position <c>at</c> read by <see cref="LanguageError"/>.</summary>
    private static readonly string[] PositionMembers = { "line", "column", "length", "stage" };

    /// <summary>Members of a candidate read by <see cref="AmbiguityCandidate"/>.</summary>
    private static readonly string[] CandidateMembers = { "rank", "ref", "program", "why" };

    private static readonly string[] SelectedFields = { "path" };

    [TestMethod]
    public void EnvelopeWithEveryOptionUsesTheMembersOfTheRequestSchema()
    {
        JsonElement schema = Schema(RequestSchema);
        JsonElement declared = schema.GetProperty("$defs").GetProperty("options");
        var options = new RunOptions
        {
            Preview = true,
            Base = "/home/user",
            Limit = 10,
            Fields = SelectedFields,
            Format = OutputFormat.Json,
            Verbosity = Verbosity.Detailed,
            Diff = DiffMode.Stat,
            DiffContext = 1,
            MaxChars = 100,
            SaveTo = "/home/user/result.json",
            Timeout = TimeSpan.FromSeconds(90),
            Session = "s1",
            Language = "pl",
            Script = true,
        };

        JsonElement envelope = EnvelopeOf(Request.FromProgram("count", NotationVariant.C), options);

        List<string> topLevel = PropertyNames(schema);
        foreach (JsonProperty member in envelope.EnumerateObject())
        {
            CollectionAssert.Contains(topLevel, member.Name);
        }

        JsonElement sent = envelope.GetProperty("options");
        CollectionAssert.AreEqual(Sorted(PropertyNames(declared)), Sorted(sent.EnumerateObject().Select(member => member.Name)));
        string timeoutPattern = declared.GetProperty("properties").GetProperty("timeout").GetProperty("pattern").GetString()!;
        StringAssert.Matches(sent.GetProperty("timeout").GetString()!, new Regex(timeoutPattern, RegexOptions.CultureInvariant));
        StringAssert.Matches(envelope.GetProperty("nurt").GetString()!, new Regex(schema.GetProperty("properties").GetProperty("nurt").GetProperty("pattern").GetString()!, RegexOptions.CultureInvariant));
    }

    [TestMethod]
    public void EnumeratedOptionsUseTheValuesOfTheSchema()
    {
        JsonElement schema = Schema(RequestSchema);
        JsonElement definitions = schema.GetProperty("$defs");
        JsonElement declared = definitions.GetProperty("options").GetProperty("properties");

        CollectionAssert.AreEquivalent(
            EnumValues(definitions.GetProperty("format")),
            Enum.GetValues<OutputFormat>().Select(value => SentOption(new RunOptions { Format = value }, "format")).ToList());
        CollectionAssert.AreEquivalent(
            EnumValues(declared.GetProperty("verbosity")),
            Enum.GetValues<Verbosity>().Select(value => SentOption(new RunOptions { Verbosity = value }, "verbosity")).ToList());
        CollectionAssert.AreEquivalent(
            EnumValues(declared.GetProperty("diff")),
            Enum.GetValues<DiffMode>().Select(value => SentOption(new RunOptions { Diff = value }, "diff")).ToList());
        CollectionAssert.AreEquivalent(
            EnumValues(schema.GetProperty("properties").GetProperty("variant")),
            Enum.GetValues<NotationVariant>().Select(value => EnvelopeOf(Request.FromProgram("count", value), null).GetProperty("variant").GetString()).ToList());
    }

    [TestMethod]
    public void EveryRequestSubjectIsOneAlternativeOfTheSchema()
    {
        JsonElement schema = Schema(RequestSchema);
        List<string> alternatives = schema.GetProperty("oneOf").EnumerateArray()
            .Select(alternative => alternative.GetProperty("required")[0].GetString()!)
            .ToList();
        Request[] requests =
        {
            Request.FromProgram("count"),
            Request.FromAst(TestSupport.Json("""{"nurt": "1.0", "program": {"statements": []}}""")),
            Request.FromCursor("c1"),
            Request.FromAccept("k1"),
        };

        List<string> subjects = requests
            .Select(request => EnvelopeOf(request, null).EnumerateObject()
                .Select(member => member.Name)
                .Single(name => alternatives.Contains(name)))
            .ToList();

        CollectionAssert.AreEquivalent(alternatives, subjects);
    }

    [TestMethod]
    public void ReportStatusCoversTheStatusesOfTheSchema()
    {
        List<string> statuses = EnumValues(Schema(ReportSchema).GetProperty("$defs").GetProperty("status"));

        List<ReportStatus> parsed = statuses.Select(Report.ParseStatus).ToList();

        CollectionAssert.DoesNotContain(parsed, ReportStatus.Unknown);
        CollectionAssert.AllItemsAreUnique(parsed);
        Assert.AreEqual(Enum.GetValues<ReportStatus>().Length - 1, parsed.Count);
        Assert.AreEqual(ReportStatus.Unknown, Report.ParseStatus("queued"));
    }

    [TestMethod]
    public void ReportAndErrorAreReadFromMembersOfTheSchema()
    {
        JsonElement schema = Schema(ReportSchema);
        JsonElement error = schema.GetProperty("$defs").GetProperty("error");
        JsonElement errorMembers = error.GetProperty("properties");

        AssertDeclared(PropertyNames(schema), ReportMembers);
        AssertDeclared(PropertyNames(error), ErrorMembers);
        AssertDeclared(PropertyNames(errorMembers.GetProperty("at")), PositionMembers);
        AssertDeclared(PropertyNames(errorMembers.GetProperty("candidates").GetProperty("items")), CandidateMembers);
    }

    [TestMethod]
    public void ErrorWrittenAsTheSchemaDescribesIsReadCompletely()
    {
        Response response = Response.FromElement(TestSupport.Json("""
            {
              "nurt": "1.0",
              "report": {
                "nurt": "1.0",
                "status": "failed",
                "kind": "find",
                "op": 12,
                "error": {
                  "code": "E_AMBIGUOUS",
                  "message": "two readings",
                  "at": { "line": 2, "column": 3, "length": 4, "stage": "5a" },
                  "fix": ["find \"src\""],
                  "help": "ambiguity",
                  "candidates": [{ "rank": 1, "ref": "k1", "program": "find \"src\"", "why": "closest" }]
                }
              },
              "rendered": { "format": "text", "text": "failed" }
            }
            """));

        LanguageError error = response.Error!;
        Assert.AreEqual(ReportStatus.Failed, response.Report.Status);
        Assert.AreEqual(12L, response.Report.Operation);
        Assert.AreEqual("E_AMBIGUOUS", error.Code);
        Assert.AreEqual("two readings", error.Message);
        Assert.AreEqual(2, error.Line);
        Assert.AreEqual(3, error.Column);
        Assert.AreEqual(4, error.Length);
        Assert.AreEqual("5a", error.Stage);
        Assert.AreEqual("find \"src\"", error.Fix.Single());
        Assert.AreEqual("ambiguity", error.Help);
        AmbiguityCandidate candidate = error.Candidates.Single();
        Assert.AreEqual(1, candidate.Rank);
        Assert.AreEqual("k1", candidate.Ref);
        Assert.AreEqual("find \"src\"", candidate.Program);
        Assert.AreEqual("closest", candidate.Why);
        Assert.AreEqual("failed", response.Text);
    }

    [TestMethod]
    public void RenderedTextIsReadAsTheResponseSchemaDescribesIt()
    {
        JsonElement definitions = Schema(RequestSchema).GetProperty("$defs");

        CollectionAssert.Contains(PropertyNames(definitions.GetProperty("response")), "rendered");
        CollectionAssert.Contains(definitions.GetProperty("rendered").GetProperty("required").EnumerateArray().Select(item => item.GetString()).ToList(), "text");
        CollectionAssert.Contains(PropertyNames(definitions.GetProperty("response")), "report");
    }

    private static JsonElement Schema(string file) =>
        TestSupport.Json(File.ReadAllText(TestSupport.RequireRepositoryFile("spec", "schemas", file)));

    private static List<string> PropertyNames(JsonElement objectSchema) =>
        objectSchema.GetProperty("properties").EnumerateObject().Select(member => member.Name).ToList();

    private static List<string> EnumValues(JsonElement schema) =>
        schema.GetProperty("enum").EnumerateArray().Select(item => item.GetString()!).ToList();

    private static List<string> Sorted(IEnumerable<string> names) => names.OrderBy(name => name, StringComparer.Ordinal).ToList();

    private static void AssertDeclared(List<string> declared, string[] read)
    {
        foreach (string member in read)
        {
            CollectionAssert.Contains(declared, member, member + " is not declared by the schema");
        }
    }

    private static JsonElement EnvelopeOf(Request request, RunOptions? options) =>
        TestSupport.Json(Encoding.UTF8.GetString(WireFormat.Envelope(request, options, new ClientOptions())));

    private static string SentOption(RunOptions options, string name) =>
        EnvelopeOf(Request.FromProgram("count"), options).GetProperty("options").GetProperty(name).GetString()!;
}
