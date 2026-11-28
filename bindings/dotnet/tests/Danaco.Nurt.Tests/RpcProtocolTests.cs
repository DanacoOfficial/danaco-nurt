// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/dotnet/tests/Danaco.Nurt.Tests/RpcProtocolTests.cs
// Responsibility:   Source file bindings/dotnet/tests/Danaco.Nurt.Tests/RpcProtocolTests.cs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

using System.Text.Json;
using Microsoft.VisualStudio.TestTools.UnitTesting;

namespace Danaco.Nurt.Tests;

/// <summary>Members of <c>nurt/event</c> notifications and of the error of a cancelled <c>run</c>, as <c>serve-protocol.md</c> defines them.</summary>
[TestClass]
public sealed class RpcProtocolTests
{
    [TestMethod]
    public void EventIsMatchedByTheRequestWrittenAsText()
    {
        Assert.AreEqual(7L, RpcErrors.RequestIdOf(TestSupport.Json("""{"request": "7", "t": "progress"}""")));
        Assert.AreEqual(8L, RpcErrors.RequestIdOf(TestSupport.Json("""{"request": 8}""")));
        Assert.AreEqual(9L, RpcErrors.RequestIdOf(TestSupport.Json("""{"request_id": 9}""")));
    }

    [TestMethod]
    public void EventWithoutAUsableRequestIsNotMatched()
    {
        Assert.IsNull(RpcErrors.RequestIdOf(TestSupport.Json("""{"request": null}""")));
        Assert.IsNull(RpcErrors.RequestIdOf(TestSupport.Json("""{"request": "scan"}""")));
        Assert.IsNull(RpcErrors.RequestIdOf(TestSupport.Json("""{"request": "-1"}""")));
        Assert.IsNull(RpcErrors.RequestIdOf(TestSupport.Json("""{"t": "log"}""")));
    }

    [TestMethod]
    public void CancelledRunAnswersWithTheResponseOfTheCancelledExecution()
    {
        JsonElement error = TestSupport.Json("""
            {"code": -32002, "message": "the request was cancelled",
             "data": {"status": "NURT_E_CANCELLED", "response": {"nurt": "1.0", "report": {"status": "failed", "error": {"code": "E_CANCELLED"}}}}}
            """);

        JsonElement response = RpcErrors.CancelledResponse(error)!.Value;

        Assert.AreEqual("E_CANCELLED", NurtResponse.FromElement(response).Error!.Code);
    }

    [TestMethod]
    public void OtherErrorsAreNotTakenForACancelledRun()
    {
        Assert.IsNull(RpcErrors.CancelledResponse(TestSupport.Json("""{"code": -32003, "data": {"response": {"report": {}}}}""")));
        Assert.IsNull(RpcErrors.CancelledResponse(TestSupport.Json("""{"code": -32002, "message": "cancelled"}""")));
        Assert.IsNull(RpcErrors.CancelledResponse(TestSupport.Json("""{"code": -32002, "data": {"response": "text"}}""")));
    }

    [TestMethod]
    public void JobEventsNameTheirJobByTheMemberId()
    {
        NurtExecutionEvent output = NurtExecutionEvent.Parse("""{"t": "job.output", "seq": 3, "id": "j7"}""");
        NurtExecutionEvent progress = NurtExecutionEvent.Parse("""{"t": "progress", "seq": 4, "id": "x"}""");

        Assert.AreEqual("j7", output.JobId);
        Assert.IsNull(progress.JobId);
    }

    [TestMethod]
    public void JsonNullIsNotAResult()
    {
        JsonElement nothing = TestSupport.Json("null");
        JsonElement empty = TestSupport.Json("{}");

        Assert.IsNull(JsonRead.WithoutNull(nothing));
        Assert.IsNull(JsonRead.WithoutNull(null));
        Assert.AreEqual(JsonValueKind.Object, JsonRead.WithoutNull(empty)!.Value.ValueKind);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
