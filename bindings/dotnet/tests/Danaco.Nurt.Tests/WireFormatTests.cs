using System;
using System.Linq;
using System.Text;
using System.Text.Json;
using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

[TestClass]
public sealed class WireFormatTests
{
    private static readonly string[] Fields = { "path", "size" };

    [TestMethod]
    public void ProgramEnvelopeHasVersionAndVariantAndOnlyTheOptionsThatAreSet()
    {
        JsonElement envelope = Envelope(Request.FromProgram("find \"/home/user\"", NotationVariant.B), null, new ClientOptions());

        Assert.AreEqual("1.0", envelope.GetProperty("nurt").GetString());
        Assert.AreEqual("find \"/home/user\"", envelope.GetProperty("program").GetString());
        Assert.AreEqual("b", envelope.GetProperty("variant").GetString());
        Assert.AreEqual(0, envelope.GetProperty("options").EnumerateObject().Count());
        Assert.IsFalse(envelope.TryGetProperty("ast", out _));
    }

    [TestMethod]
    public void ClientLanguageAndSessionApplyWhenTheCallSetsNone()
    {
        var client = new ClientOptions { Language = "pl", Session = "client" };

        JsonElement inherited = Envelope(Request.FromProgram("x"), null, client).GetProperty("options");
        JsonElement overridden = Envelope(Request.FromProgram("x"), new RunOptions { Language = "en" }, client).GetProperty("options");

        Assert.AreEqual("pl", inherited.GetProperty("language").GetString());
        Assert.AreEqual("client", inherited.GetProperty("session").GetString());
        Assert.AreEqual("en", overridden.GetProperty("language").GetString());
    }

    [TestMethod]
    public void EveryOptionIsWrittenWithItsSchemaName()
    {
        var options = new RunOptions
        {
            Preview = true,
            Base = "/home/user",
            Limit = 10,
            Fields = Fields,
            Format = OutputFormat.Compact,
            Verbosity = Verbosity.Detailed,
            Diff = DiffMode.Stat,
            DiffContext = 1,
            MaxChars = 200,
            SaveTo = "/home/user/result.json",
            Timeout = TimeSpan.FromMinutes(2),
            Session = "run-1",
            Language = "pl",
            Script = true,
        };

        JsonElement sent = Envelope(Request.FromProgram("x"), options, new ClientOptions { Session = "client" }).GetProperty("options");

        Assert.IsTrue(sent.GetProperty("preview").GetBoolean());
        Assert.AreEqual("/home/user", sent.GetProperty("base").GetString());
        Assert.AreEqual(10, sent.GetProperty("limit").GetInt32());
        Assert.AreEqual("size", sent.GetProperty("fields")[1].GetString());
        Assert.AreEqual("compact", sent.GetProperty("format").GetString());
        Assert.AreEqual("detailed", sent.GetProperty("verbosity").GetString());
        Assert.AreEqual("stat", sent.GetProperty("diff").GetString());
        Assert.AreEqual(1, sent.GetProperty("diff_context").GetInt32());
        Assert.AreEqual(200, sent.GetProperty("max_chars").GetInt32());
        Assert.AreEqual("/home/user/result.json", sent.GetProperty("save_to").GetString());
        Assert.AreEqual("120000ms", sent.GetProperty("timeout").GetString());
        Assert.AreEqual("run-1", sent.GetProperty("session").GetString());
        Assert.AreEqual("pl", sent.GetProperty("language").GetString());
        Assert.IsTrue(sent.GetProperty("script").GetBoolean());
        Assert.IsFalse(sent.TryGetProperty("throw_on_language_error", out _));
    }

    [TestMethod]
    public void SubjectsAreExclusive()
    {
        JsonElement ast = TestSupport.Json("""{"nurt": "1.0", "program": []}""");

        JsonElement fromAst = Envelope(Request.FromAst(ast), null, new ClientOptions());
        JsonElement fromCursor = Envelope(Request.FromCursor("c1"), null, new ClientOptions());
        JsonElement fromAccept = Envelope(Request.FromAccept("k1"), null, new ClientOptions());

        Assert.IsFalse(fromAst.TryGetProperty("program", out _));
        Assert.AreEqual("1.0", fromAst.GetProperty("ast").GetProperty("nurt").GetString());
        Assert.AreEqual("c1", fromCursor.GetProperty("cursor").GetString());
        Assert.AreEqual("k1", fromAccept.GetProperty("accept").GetString());
        Assert.IsFalse(fromAccept.TryGetProperty("variant", out _));
    }

    [TestMethod]
    public void DurationIsWrittenInMilliseconds()
    {
        Assert.AreEqual("1500ms", WireFormat.Duration(TimeSpan.FromMilliseconds(1500)));
        Assert.AreEqual("0ms", WireFormat.Duration(TimeSpan.Zero));
        Assert.ThrowsException<ArgumentOutOfRangeException>(() => WireFormat.Duration(TimeSpan.FromSeconds(-1)));
    }

    [TestMethod]
    public void CompactJsonFitsOnOneLine()
    {
        JsonElement value = TestSupport.Json("{\n  \"a\": [1,\n 2],\n  \"b\": \"x\\ny\"\n}");

        string compact = Encoding.UTF8.GetString(WireFormat.Compact(value));

        Assert.AreEqual("{\"a\":[1,2],\"b\":\"x\\ny\"}", compact);
    }

    [TestMethod]
    public void VariantsMatchTheHeaderValues()
    {
        Assert.AreEqual(1, WireFormat.NativeVariant(NotationVariant.A));
        Assert.AreEqual(2, WireFormat.NativeVariant(NotationVariant.B));
        Assert.AreEqual(3, WireFormat.NativeVariant(NotationVariant.C));
        Assert.AreEqual("c", WireFormat.VariantName(NotationVariant.C));
        Assert.ThrowsException<ArgumentOutOfRangeException>(() => WireFormat.VariantName((NotationVariant)7));
    }

    [TestMethod]
    public void TextWithZeroCharacterIsRejected()
    {
        Assert.ThrowsException<ArgumentException>(() => Utf8.ToNative("a\0b", "program"));
        CollectionAssert.AreEqual(new byte[] { 0x61, 0xC5, 0x82, 0 }, Utf8.ToNative("a\u0142", "program"));
        Assert.IsNull(Utf8.ToNative(null, "topic"));
    }

    [TestMethod]
    public void ArgumentsAreQuotedForTheCommandLine()
    {
        Assert.AreEqual("serve", Quote("serve"));
        Assert.AreEqual("\"\"", Quote(string.Empty));
        Assert.AreEqual("\"a b\"", Quote("a b"));
        Assert.AreEqual("\"say \\\"x\\\"\"", Quote("say \"x\""));
        Assert.AreEqual("\"C:\\dir name\\\\\"", Quote("C:\\dir name\\"));
        Assert.AreEqual("C:\\dir\\file", Quote("C:\\dir\\file"));
    }

    [TestMethod]
    public void RpcErrorCodesMapToHeaderStatuses()
    {
        Assert.AreEqual(CallStatus.InvalidArgument, RpcErrors.ToStatus(-32003));
        Assert.AreEqual(CallStatus.Io, RpcErrors.ToStatus(-32005));
        Assert.AreEqual(CallStatus.InvalidArgument, RpcErrors.ToStatus(RpcErrors.ParseError));
        Assert.AreEqual(CallStatus.InvalidArgument, RpcErrors.ToStatus(RpcErrors.InvalidParams));
        Assert.AreEqual(CallStatus.AbiMismatch, RpcErrors.ToStatus(RpcErrors.MethodNotFound));
        Assert.AreEqual(CallStatus.Internal, RpcErrors.ToStatus(RpcErrors.InternalError));
        Assert.AreEqual(CallStatus.Internal, RpcErrors.ToStatus(-31000));
        Assert.AreEqual((CallStatus)42, RpcErrors.ToStatus(-32042));
        Assert.AreEqual("42", CallException.GetStatusName((CallStatus)42));
    }

    private static string Quote(string argument)
    {
        var line = new StringBuilder();
        CommandLine.AppendQuoted(line, argument);
        return line.ToString();
    }

    private static JsonElement Envelope(Request request, RunOptions? options, ClientOptions client) =>
        TestSupport.Json(Encoding.UTF8.GetString(WireFormat.Envelope(request, options, client)));
}
