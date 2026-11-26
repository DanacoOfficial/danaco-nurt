// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.internal;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;
import pl.danacogroup.nurt.CallStatus;
import pl.danacogroup.nurt.NurtCallException;

/** Reading of the messages of {@code nurt serve} that need no process. */
class ProcessBackendTest {
    @Test
    void cancelledRunCarriesTheResponseOfTheCancelledExecution() {
        Object error = Json.parse("{\"code\":-32002,\"message\":\"the request was cancelled\","
                + "\"data\":{\"status\":\"NURT_E_CANCELLED\",\"response\":{\"report\":{\"status\":\"failed\"}}}}");
        assertEquals(Json.parse("{\"report\":{\"status\":\"failed\"}}"), ProcessBackend.cancelledResponse(error));
    }

    @Test
    void otherErrorsCarryNoResponse() {
        String response = "\"data\":{\"response\":{\"report\":{}}}";
        assertNull(ProcessBackend.cancelledResponse(Json.parse("{\"code\":-32003," + response + "}")));
        assertNull(ProcessBackend.cancelledResponse(Json.parse("{\"code\":-32002,\"message\":\"cancelled\"}")));
        assertNull(ProcessBackend.cancelledResponse(Json.parse("{\"code\":-32002,\"data\":{\"response\":\"text\"}}")));
        assertNull(ProcessBackend.cancelledResponse(Json.parse("{\"message\":\"no code\"," + response + "}")));
        assertNull(ProcessBackend.cancelledResponse("not an object"));
    }

    @Test
    void requestIdsAreNumbersOrNumericText() {
        assertEquals(7L, ProcessBackend.requestId(7L));
        assertEquals(8L, ProcessBackend.requestId("8"));
        assertNull(ProcessBackend.requestId("scan"));
        assertNull(ProcessBackend.requestId(-1L));
        assertNull(ProcessBackend.requestId(null));
    }

    @Test
    void applicationErrorsKeepTheStatusOfTheCInterface() {
        for (CallStatus status : CallStatus.values()) {
            Object error = Json.parse("{\"code\":" + (-32000 - status.code()) + ",\"message\":\"m\"}");
            assertEquals(status, ProcessBackend.rpcError(error).status(), status.cName());
        }
        assertEquals(NurtCallException.Kind.INTERNAL,
                ProcessBackend.rpcError(Json.parse("{\"code\":-32050,\"message\":\"m\"}")).kind());
        assertEquals(NurtCallException.Kind.INTERNAL, ProcessBackend.rpcError(Json.parse("{\"message\":\"m\"}")).kind());
    }

    @Test
    void protocolOfTheSameMajorVersionIsCompatible() {
        assertTrue(ProcessBackend.isCompatible("nurt-rpc/1"));
        assertTrue(ProcessBackend.isCompatible("nurt-rpc/1.4"));
        assertFalse(ProcessBackend.isCompatible("nurt-rpc/2"));
        assertFalse(ProcessBackend.isCompatible("nurt-rpc/10"));
        assertFalse(ProcessBackend.isCompatible(null));
    }
}
