// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/test/java/pl/danacogroup/nurt/EngineTest.java
// Responsibility:   Tests of the process backend against the real engine nurt serve --stdio.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Assumptions;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

/**
 * Every method of the process backend on the real engine. The tests need the variable
 * {@code NURT_TEST_EXECUTABLE}, which names the {@code nurt} executable; without it they are skipped.
 */
class EngineTest {
    private static final Duration WAIT = Duration.ofSeconds(60);

    @TempDir
    Path temp;

    private NurtClient client;

    @BeforeEach
    void open() {
        String executable = System.getenv("NURT_TEST_EXECUTABLE");
        Assumptions.assumeTrue(executable != null && !executable.isEmpty(), "NURT_TEST_EXECUTABLE is not set");
        client = NurtClient.create(new NurtClientOptions()
                .backend(BackendKind.PROCESS)
                .executable(Path.of(executable))
                .environment(Map.of("NURT_STATE_DIR", temp.resolve("state").toString()))
                .requestTimeout(WAIT));
    }

    @AfterEach
    void close() {
        if (client != null) {
            client.close();
        }
    }

    private String listing(int files) throws IOException {
        Path directory = Files.createDirectories(temp.resolve("files"));
        for (int index = 0; index < files; index++) {
            Files.writeString(directory.resolve("file-" + index + ".txt"), "text\n");
        }
        return "list \"" + directory.toString().replace('\\', '/') + "\"";
    }

    @Test
    void infoReportsTheFormats() {
        Map<String, Object> info = client.info();
        @SuppressWarnings("unchecked")
        Map<String, Object> formats = (Map<String, Object>) info.get("formats");
        assertEquals("1.1", formats.get("ast"));
        assertEquals("1.1", formats.get("bound"));
        assertInstanceOf(String.class, info.get("version"));
    }

    @Test
    void catalogSelectionsAreAnswered() {
        assertEquals("nurt-catalog-result/1", client.catalog().get("format"));
        assertFalse(((List<?>) client.catalog(Map.of("group", "core")).get("commands")).isEmpty());
        @SuppressWarnings("unchecked")
        Map<String, Object> command = (Map<String, Object>) client.catalog(Map.of("command", "list")).get("command");
        assertEquals("list", command.get("name"));
    }

    @Test
    void schemaIsServedAndAnUnknownOneIsAnArgumentError() {
        assertEquals("nurt-ast/1", client.schema("ast").get("title"));
        NurtCallException failure = assertThrows(NurtCallException.class, () -> client.schema("unknown"));
        assertEquals(NurtCallException.Kind.INVALID_ARGUMENT, failure.kind());
    }

    @Test
    void describeAndHelpReturnText() {
        String description = client.describe();
        assertTrue(description.contains("PROGRAM"), description);
        assertFalse(description.startsWith("{"), description);
        NurtLanguageException unknown = assertThrows(NurtLanguageException.class,
                () -> client.describe("no-such-command", null));
        assertEquals("E_UNKNOWN_NAME", unknown.error().code());
        assertTrue(client.help().startsWith("GROUP"));
        assertFalse(client.help("list").isEmpty());
    }

    @Test
    void aShortenedResultCarriesAContinuation() throws IOException {
        NurtResponse response = client.run(Request.program(listing(5)).limit(2));
        assertTrue(response.ok(), response.toString());
        assertEquals("list", response.report().get("kind"));
        @SuppressWarnings("unchecked")
        Map<String, Object> continuation = (Map<String, Object>) response.report().get("continuation");
        assertEquals(3L, ((Number) continuation.get("remaining")).longValue());
        NurtResponse next = client.run(Request.program("next"));
        assertTrue(next.ok(), next.toString());
        assertEquals("next", next.report().get("kind"));
        assertTrue(client.page(response.nextCursor().orElseThrow()).ok());
    }

    @Test
    void aLanguageErrorIsPartOfTheResponse() {
        NurtResponse response = client.run(Request.program("lst \"/tmp\""));
        assertFalse(response.ok());
        assertEquals("E_UNKNOWN_COMMAND", response.error().orElseThrow().code());
        assertThrows(NurtLanguageException.class, response::orThrow);
        assertEquals("E_ARG", client.page("malformed").error().orElseThrow().code());
    }

    @Test
    void eventsPrecedeTheResponse() throws IOException {
        Execution execution = client.start(Request.program(listing(1)));
        List<String> kinds = new ArrayList<>();
        execution.events().subscribe(new java.util.concurrent.Flow.Subscriber<>() {
            @Override
            public void onSubscribe(java.util.concurrent.Flow.Subscription subscription) {
                subscription.request(Long.MAX_VALUE);
            }

            @Override
            public void onNext(NurtEvent event) {
                synchronized (kinds) {
                    kinds.add(event.kind());
                }
            }

            @Override
            public void onError(Throwable error) {
            }

            @Override
            public void onComplete() {
            }
        });
        assertTrue(execution.await(WAIT).ok());
        execution.close();
        synchronized (kinds) {
            assertEquals("exec.started", kinds.get(0));
            assertEquals("exec.finished", kinds.get(kinds.size() - 1));
        }
    }

    @Test
    void checkParsePrintAndCorrectReturnTheResults() {
        assertNotNull(client.check(Request.program("list \"/tmp\"")).get("bound"));
        Map<String, Object> parsed = client.parse("list \"/tmp\"", Request.Variant.A);
        @SuppressWarnings("unchecked")
        Map<String, Object> ast = (Map<String, Object>) parsed.get("ast");
        assertEquals("1.1", ast.get("nurt"));
        assertTrue(client.print(ast, Request.Variant.A).startsWith("list"));
        assertNotNull(client.parse("list (", Request.Variant.A).get("report"));
        assertEquals("list \"/tmp\"", client.correct("list \"/tmp\"", Request.Variant.A).get("program"));
        Map<String, Object> quoted = client.correct("find '/tmp'", Request.Variant.A);
        assertEquals("find \"/tmp\"", quoted.get("program"));
        assertFalse(((List<?>) quoted.get("corrections")).isEmpty());
    }

    @Test
    void historyAndSpaceAreAnswered() {
        assertTrue(client.history().get("operations") instanceof List<?>);
        assertTrue(client.history(Map.of("limit", 3L)).get("operations") instanceof List<?>);
        assertTrue(client.space().get("volumes") instanceof List<?>);
    }

    @Test
    void theOptionsPrivilegeAndAsUserAreAccepted() throws IOException {
        for (Request.Privilege privilege : List.of(Request.Privilege.ACCOUNT, Request.Privilege.AUTO)) {
            assertTrue(client.run(Request.program(listing(1)).privilege(privilege)).ok());
        }
        NurtResponse response = client.run(Request.program(listing(1)).asUser("root"));
        if (response.ok()) {
            assertEquals("root", response.report().get("as_user"));
        } else {
            assertTrue(response.error().orElseThrow().code().startsWith("E_"));
        }
    }
}
