using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

[TestClass]
public sealed class ResponseTests
{
    [TestMethod]
    public void ReportFieldsAndRenderedTextAreRead()
    {
        Response response = Response.Parse("""
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
        Response response = Response.Parse("""{"nurt": "1.1", "report": {"nurt": "1.1", "status": "deferred", "kind": "run"}}""");

        Assert.AreEqual(ReportStatus.Unknown, response.Report.Status);
        Assert.AreEqual("deferred", response.Report.StatusText);
        Assert.IsNull(response.Text);
    }

    [TestMethod]
    public void ResponseWithoutReportIsCallFailure()
    {
        CallException exception = Assert.ThrowsException<CallException>(() => Response.Parse("""{"nurt": "1.0"}"""));

        Assert.AreEqual(CallStatus.Internal, exception.Status);
    }

    [TestMethod]
    public void TextThatIsNotJsonIsCallFailure()
    {
        CallException exception = Assert.ThrowsException<CallException>(() => Response.Parse("{"));

        Assert.AreEqual(CallStatus.Internal, exception.Status);
    }

    [TestMethod]
    public void LanguageExceptionCarriesTheResponse()
    {
        Response response = Response.Parse("""{"nurt": "1.0", "report": {"nurt": "1.0", "status": "failed", "kind": "find", "error": {"code": "E_PATH_RELATIVE", "message": "path is relative"}}}""");

        var exception = new LanguageException(response);

        Assert.AreSame(response, exception.Response);
        Assert.AreEqual("E_PATH_RELATIVE", exception.Error!.Code);
        Assert.AreEqual("E_PATH_RELATIVE: path is relative", exception.Message);
        Assert.AreEqual(0, exception.Error.Fix.Count);
        Assert.IsNull(exception.Error.Line);
    }
}
