// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.time.Duration;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import org.junit.jupiter.api.Test;
import pl.danacogroup.nurt.internal.Json;

class RequestResponseTest {
    @Test
    void minimalProgramHasOnlyVersionAndText() {
        assertEquals("{\"nurt\":\"1.0\",\"program\":\"find \\\"/srv/app\\\"\"}",
                Request.program("find \"/srv/app\"").toJson());
    }

    @Test
    void everyOptionUsesItsWireName() {
        Request request = Request.program("find \"/srv/app\"")
                .variant(Request.Variant.B)
                .preview(true)
                .base("/srv/app")
                .limit(10)
                .fields(List.of("path", "size"))
                .format(Request.Format.MARKDOWN)
                .verbosity(Request.Verbosity.DETAILED)
                .diff(Request.DiffMode.STAT)
                .diffContext(5)
                .maxChars(400)
                .saveTo("/home/user/out.txt")
                .timeout(Duration.ofMillis(1500))
                .session("s-1")
                .language(Request.Language.PL)
                .script(true)
                .option("future_option", "x");
        assertEquals(Json.parse("{\"nurt\":\"1.0\",\"program\":\"find \\\"/srv/app\\\"\",\"variant\":\"b\","
                + "\"options\":{\"preview\":true,\"base\":\"/srv/app\",\"limit\":10,\"fields\":[\"path\",\"size\"],"
                + "\"format\":\"markdown\",\"verbosity\":\"detailed\",\"diff\":\"stat\",\"diff_context\":5,"
                + "\"max_chars\":400,\"save_to\":\"/home/user/out.txt\",\"timeout\":\"1500ms\",\"session\":\"s-1\","
                + "\"language\":\"pl\",\"script\":true,\"future_option\":\"x\"}}"), request.toMap());
    }

    @Test
    void wholeSecondsAreWrittenInSeconds() {
        Map<String, Object> options = Json.object(Request.program("x").timeout(Duration.ofMinutes(2)).toMap().get("options"));
        assertEquals("120s", options.get("timeout"));
    }

    @Test
    void eachInputUsesItsOwnMember() {
        assertEquals("c_1", Request.cursor("c_1").toMap().get("cursor"));
        assertEquals("c2", Request.accept("c2").toMap().get("accept"));
        Map<String, Object> tree = Map.of("nurt", "1.0");
        assertEquals(tree, Request.ast(tree).toMap().get("ast"));
        assertThrows(NullPointerException.class, () -> Request.program(null));
    }

    @Test
    void responseWithoutReportIsAProtocolError() {
        NurtCallException error = assertThrows(NurtCallException.class, () -> NurtResponse.parse("{\"nurt\":\"1.0\"}"));
        assertEquals(NurtCallException.Kind.PROTOCOL, error.kind());
        assertEquals(NurtCallException.Kind.PROTOCOL,
                assertThrows(NurtCallException.class, () -> NurtResponse.parse("{\"report\":")).kind());
    }

    @Test
    void responseExposesTextCursorAndStatus() {
        NurtResponse response = NurtResponse.parse("{\"nurt\":\"1.0\",\"report\":{\"status\":\"ok\",\"next_cursor\":\"c_9\"},"
                + "\"rendered\":{\"format\":\"text\",\"text\":\"3 files\"}}");
        assertTrue(response.ok());
        assertEquals(Optional.of("3 files"), response.text());
        assertEquals(Optional.of("c_9"), response.nextCursor());
        assertEquals(Optional.of("ok"), response.status());
        assertEquals(response, NurtResponse.parse(response.json()));
        assertEquals(response, response.orThrow());
    }

    @Test
    void languageErrorExposesItsFields() {
        NurtResponse response = NurtResponse.parse("{\"nurt\":\"1.0\",\"report\":{\"status\":\"failed\",\"error\":{"
                + "\"code\":\"E_SYNTAX\",\"message\":\"unclosed list\",\"stage\":\"0\","
                + "\"at\":{\"line\":1,\"column\":27,\"length\":1},\"fix\":[\"a\",\"b\"],\"help\":\"lists\"}}}");
        assertFalse(response.ok());
        LanguageError error = response.error().orElseThrow();
        assertEquals("E_SYNTAX", error.code());
        assertEquals("unclosed list", error.message());
        assertEquals(Optional.of("0"), error.stage());
        assertEquals(27L, error.at().orElseThrow().get("column"));
        assertEquals(List.of("a", "b"), error.fix());
        assertEquals(Optional.of("lists"), error.help());
        assertEquals("E_SYNTAX: unclosed list", error.toString());
        assertEquals("E_SYNTAX", assertThrows(NurtLanguageException.class, response::orThrow).error().code());
    }

    @Test
    void eventExposesTypeSequenceAndJob() {
        NurtEvent event = new NurtEvent(Json.object(Json.parse("{\"t\":\"job.progress\",\"seq\":4,\"job_id\":\"j-2\"}")));
        assertEquals("job.progress", event.kind());
        assertEquals(4L, event.seq().orElseThrow());
        assertEquals(Optional.of("j-2"), event.jobId());
        NurtEvent empty = new NurtEvent(Map.of());
        assertEquals("", empty.kind());
        assertTrue(empty.seq().isEmpty());
    }

    @Test
    void statusCodesMatchTheCInterface() {
        int expected = 1;
        for (CallStatus status : CallStatus.values()) {
            assertEquals(expected++, status.code());
            assertEquals(Optional.of(status), CallStatus.fromCode(status.code()));
            assertEquals(Optional.of(status), CallStatus.fromCName(status.cName()));
            assertEquals(status, NurtCallException.Kind.of(status).status());
        }
        assertEquals("NURT_E_ARG", CallStatus.ARG.cName());
        assertTrue(CallStatus.fromCode(0).isEmpty());
        assertTrue(CallStatus.fromCode(7).isEmpty());
        assertEquals(CallStatus.IO, NurtCallException.Kind.NOT_FOUND.status());
        assertEquals(CallStatus.IO, NurtCallException.Kind.DISCONNECTED.status());
        assertEquals(CallStatus.INTERNAL, NurtCallException.Kind.PROTOCOL.status());
    }
}
