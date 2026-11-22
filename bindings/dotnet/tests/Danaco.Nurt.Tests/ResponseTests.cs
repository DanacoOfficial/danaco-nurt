// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

[TestClass]
public sealed class ResponseTests
{
    [TestMethod]
    public void ReportFieldsAndRenderedTextAreRead()
    {
        NurtResponse response = NurtResponse.Parse("""
            {"nurt": "1.0",
             "report": {"nurt": "1.0", "status": "rolled_back", "kind": "replace", "op": 12, "next_cursor": "c9"},
             "rendered": {"format": "text", "text": "rolled back"}}
            """);

        Assert.AreEqual(ReportStatus.RolledBack, response.Report.Status);
        Assert.AreEqual("rolled_back", response.Report.StatusText);
        Assert.AreEqual("replace", response.Report.Kind);
        Assert.AreEqual(12L, response.Report.Operation);
        Assert.AreEqual("c9", response.NextCursor);
        Assert.AreEqual("rolled back", response.Text);
        Assert.IsTrue(response.Ok);
        StringAssert.Contains(response.Raw, "rolled_back");
    }

    [TestMethod]
    public void StatusOfALaterVersionIsUnknown()
    {
        NurtResponse response = NurtResponse.Parse("""{"nurt": "1.1", "report": {"nurt": "1.1", "status": "deferred", "kind": "run"}}""");

        Assert.AreEqual(ReportStatus.Unknown, response.Report.Status);
        Assert.AreEqual("deferred", response.Report.StatusText);
        Assert.IsNull(response.Text);
    }

    [TestMethod]
    public void ResponseWithoutReportIsCallFailure()
    {
        NurtCallException exception = Assert.ThrowsException<NurtCallException>(() => NurtResponse.Parse("""{"nurt": "1.0"}"""));

        Assert.AreEqual(CallStatus.Internal, exception.Status);
    }

    [TestMethod]
    public void TextThatIsNotJsonIsCallFailure()
    {
        NurtCallException exception = Assert.ThrowsException<NurtCallException>(() => NurtResponse.Parse("{"));

        Assert.AreEqual(CallStatus.Internal, exception.Status);
    }

    [TestMethod]
    public void LanguageExceptionCarriesTheResponse()
    {
        NurtResponse response = NurtResponse.Parse("""{"nurt": "1.0", "report": {"nurt": "1.0", "status": "failed", "kind": "find", "error": {"code": "E_PATH_RELATIVE", "message": "path is relative"}}}""");

        var exception = new NurtLanguageException(response);

        Assert.AreSame(response, exception.Response);
        Assert.AreEqual("E_PATH_RELATIVE", exception.Error!.Code);
        Assert.AreEqual("E_PATH_RELATIVE: path is relative", exception.Message);
        Assert.AreEqual(0, exception.Error.Fix.Count);
        Assert.IsNull(exception.Error.Line);
    }
}
