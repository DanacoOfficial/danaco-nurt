// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/test/java/pl/danacogroup/nurt/ProcessClientTest.java
// Responsibility:   Source file bindings/java/nurt/src/test/java/pl/danacogroup/nurt/ProcessClientTest.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Flow;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;
import pl.danacogroup.nurt.testing.FakeServer;

class ProcessClientTest {
    private static final Duration WAIT = Duration.ofSeconds(30);

    @TempDir
    Path temp;

    private static List<String> serverCommand() {
        return List.of(Path.of(System.getProperty("java.home"), "bin", "java").toString(),
                "-cp", System.getProperty("java.class.path"), FakeServer.class.getName());
    }

    private static NurtClientOptions options() {
        return new NurtClientOptions().backend(BackendKind.PROCESS).command(serverCommand());
    }

    private Path log() {
        return temp.resolve("methods.log");
    }

    private List<String> loggedMethods() throws IOException {
        return Files.exists(log()) ? Files.readAllLines(log()) : List.of();
    }

    /** Subscriber that requests every event and records them. */
    private static final class Collector implements Flow.Subscriber<NurtEvent> {
        final List<NurtEvent> events = new CopyOnWriteArrayList<>();
        final CompletableFuture<NurtEvent> first = new CompletableFuture<>();
        final CompletableFuture<Void> done = new CompletableFuture<>();

        @Override
        public void onSubscribe(Flow.Subscription subscription) {
            subscription.request(Long.MAX_VALUE);
        }

        @Override
        public void onNext(NurtEvent event) {
            events.add(event);
            first.complete(event);
        }

        @Override
        public void onError(Throwable error) {
            done.completeExceptionally(error);
        }

        @Override
        public void onComplete() {
            done.complete(null);
        }
    }

    @Test
    void runReturnsTheResponse() {
        try (NurtClient client = NurtClient.create(options())) {
            assertEquals(BackendKind.PROCESS, client.backend());
            NurtResponse response = client.run(Request.program("find \"/srv/app\"").limit(5));
            assertTrue(response.ok());
            assertEquals("find \"/srv/app\"", response.text().orElseThrow());
            assertEquals("ok", response.status().orElseThrow());
        }
    }

    @Test
    void initializeCarriesTheSettings() throws IOException {
        NurtClientOptions options = options()
                .environment(Map.of("FAKE_LOG", log().toString()))
                .session("s-7")
                .language(Request.Language.PL);
        try (NurtClient client = NurtClient.create(options)) {
            assertEquals("info", client.info().get("method"));
        }
        assertEquals(List.of("initialize", "info", "shutdown"), loggedMethods());
    }

    @Test
    void otherMajorProtocolVersionIsRejected() {
        NurtClientOptions options = options().environment(Map.of("FAKE_PROTOCOL", "nurt-rpc/2"));
        NurtCallException error = assertThrows(NurtCallException.class, () -> NurtClient.create(options));
        assertEquals(NurtCallException.Kind.INCOMPATIBLE, error.kind());
        assertEquals(CallStatus.ABI, error.status());
    }

    @Test
    void newerMinorProtocolVersionIsAccepted() {
        NurtClientOptions options = options().environment(Map.of("FAKE_PROTOCOL", "nurt-rpc/1.3"));
        try (NurtClient client = NurtClient.create(options)) {
            assertTrue(client.run(Request.program("x")).ok());
        }
    }

    @Test
    void languageErrorIsNotAnException() {
        try (NurtClient client = NurtClient.create(options())) {
            NurtResponse response = client.run(Request.program("language"));
            assertFalse(response.ok());
            LanguageError error = response.error().orElseThrow();
            assertEquals("E_SYNTAX", error.code());
            assertEquals(List.of("find \"/srv/app\" glob: [\"*.go\"]"), error.fix());
            NurtLanguageException raised = assertThrows(NurtLanguageException.class, response::orThrow);
            assertEquals("E_SYNTAX", raised.error().code());
        }
    }

    @Test
    void applicationErrorsCarryTheirStatus() {
        try (NurtClient client = NurtClient.create(options())) {
            for (CallStatus status : CallStatus.values()) {
                long code = -32000 - status.code();
                NurtCallException error = assertThrows(NurtCallException.class,
                        () -> client.run(Request.program("error:" + code)));
                assertEquals(status, error.status(), "code " + code);
            }
            NurtCallException unknownMethod = assertThrows(NurtCallException.class,
                    () -> client.run(Request.program("error:-32601")));
            assertEquals(NurtCallException.Kind.PROTOCOL, unknownMethod.kind());
            NurtCallException invalidParams = assertThrows(NurtCallException.class,
                    () -> client.run(Request.program("error:-32602")));
            assertEquals(CallStatus.ARG, invalidParams.status());
        }
    }

    @Test
    void parallelCallsFromManyThreadsAreMatchedById() throws Exception {
        int threads = 8;
        int callsPerThread = 25;
        ExecutorService pool = Executors.newFixedThreadPool(threads);
        try (NurtClient client = NurtClient.create(options())) {
            List<Future<?>> results = new ArrayList<>();
            for (int t = 0; t < threads; t++) {
                int thread = t;
                results.add(pool.submit(() -> {
                    for (int i = 0; i < callsPerThread; i++) {
                        String program = "program-" + thread + "-" + i;
                        assertEquals(program, client.run(Request.program(program)).text().orElseThrow());
                    }
                    return null;
                }));
            }
            for (Future<?> result : results) {
                result.get(WAIT.toSeconds(), TimeUnit.SECONDS);
            }
        } finally {
            pool.shutdownNow();
        }
    }

    @Test
    void answersInReverseOrderReachTheirCallers() throws Exception {
        int held = 6;
        try (NurtClient client = NurtClient.create(options())) {
            List<CompletableFuture<NurtResponse>> responses = new ArrayList<>();
            for (int i = 0; i < held; i++) {
                responses.add(client.runAsync(Request.program("hold:" + held + "/" + i)));
            }
            for (int i = 0; i < held; i++) {
                NurtResponse response = responses.get(i).get(WAIT.toSeconds(), TimeUnit.SECONDS);
                assertEquals("hold:" + held + "/" + i, response.report().get("kind"));
            }
        }
    }

    @Test
    void eventsReachTheirExecutionAndOthersReachTheClient() throws Exception {
        try (NurtClient client = NurtClient.create(options())) {
            Collector outside = new Collector();
            client.events().subscribe(outside);
            try (Execution execution = client.start(Request.program("events"))) {
                assertTrue(execution.await(WAIT).ok());
                Collector own = new Collector();
                execution.events().subscribe(own);
                own.done.get(WAIT.toSeconds(), TimeUnit.SECONDS);
                List<String> kinds = own.events.stream().map(NurtEvent::kind).toList();
                assertEquals(List.of("progress", "progress", "progress", "exec.finished"), kinds);
                assertEquals(4L, own.events.get(3).seq().orElseThrow());
            }
            NurtEvent job = outside.first.get(WAIT.toSeconds(), TimeUnit.SECONDS);
            assertEquals("j-1", job.jobId().orElseThrow());
        }
    }

    @Test
    void cancelEndsASlowExecution() {
        try (NurtClient client = NurtClient.create(options())) {
            Execution execution = client.start(Request.program("slow"));
            NurtCallException running = assertThrows(NurtCallException.class,
                    () -> execution.await(Duration.ofMillis(100)));
            assertEquals(NurtCallException.Kind.TIMEOUT, running.kind());
            assertFalse(execution.isDone());
            execution.cancel();
            assertEquals("rolled_back", execution.await(WAIT).status().orElseThrow());
            assertTrue(execution.isDone());
        }
    }

    @Test
    void requestTimeoutFailsTheCallAndCancelsIt() throws IOException {
        NurtClientOptions options = options()
                .environment(Map.of("FAKE_LOG", log().toString()))
                .requestTimeout(Duration.ofSeconds(2));
        try (NurtClient client = NurtClient.create(options)) {
            NurtCallException error = assertThrows(NurtCallException.class, () -> client.run(Request.program("slow")));
            assertEquals(NurtCallException.Kind.TIMEOUT, error.kind());
            assertEquals(CallStatus.TIMEOUT, error.status());
            assertTrue(client.run(Request.program("after")).ok());
        }
        assertTrue(loggedMethods().contains("cancel"));
    }

    @Test
    void endedServerFailsRequestsInFlightAndIsRestarted() throws IOException {
        NurtClientOptions options = options().environment(Map.of("FAKE_LOG", log().toString()));
        try (NurtClient client = NurtClient.create(options)) {
            NurtCallException error = assertThrows(NurtCallException.class, () -> client.run(Request.program("crash")));
            assertEquals(NurtCallException.Kind.DISCONNECTED, error.kind());
            assertTrue(error.getMessage().contains("exit code 3"), error.getMessage());
            assertTrue(client.run(Request.program("again")).ok());
        }
        long initializations = loggedMethods().stream().filter("initialize"::equals).count();
        assertEquals(2, initializations);
    }

    @Test
    void invalidJsonFromTheServerClosesTheConnection() {
        try (NurtClient client = NurtClient.create(options())) {
            NurtCallException error = assertThrows(NurtCallException.class, () -> client.run(Request.program("garble")));
            assertEquals(NurtCallException.Kind.PROTOCOL, error.kind());
            assertTrue(client.run(Request.program("again")).ok());
        }
    }

    @Test
    void methodsSendTheirParameters() {
        try (NurtClient client = NurtClient.create(options())) {
            Map<String, Object> tree = Map.of("nurt", "1.0");
            assertEquals(Map.of("program", "find(\"/srv\")", "variant", "b"),
                    client.parse("find(\"/srv\")", Request.Variant.B).get("params"));
            assertEquals("print:{\"ast\":{\"nurt\":\"1.0\"},\"variant\":\"c\"}", client.print(tree, Request.Variant.C));
            assertEquals(Map.of("program", "find /srv", "variant", "a"),
                    client.correct("find /srv", Request.Variant.A).get("params"));
            assertEquals("check", client.check(Request.program("x")).get("method"));
            assertEquals("describe:{\"subject\":\"core\"}", client.describe());
            assertEquals("describe:{\"subject\":\"git\",\"language\":\"pl\"}",
                    client.describe("git", Request.Language.PL));
            assertEquals("help:{}", client.help());
            assertEquals("help:{\"topic\":\"lists\"}", client.help("lists"));
            assertEquals(Map.of(), client.catalog().get("params"));
            assertEquals(Map.of("group", "git"), client.catalog(Map.of("group", "git")).get("params"));
            assertEquals(Map.of("name", "report"), client.schema("report").get("params"));
            assertEquals(Map.of(), client.history().get("params"));
            assertEquals(Map.of("limit", 3L), client.history(Map.of("limit", 3L)).get("params"));
            assertEquals(Map.of(), client.space().get("params"));
            assertEquals(Map.of("volume", "/srv"), client.space("/srv").get("params"));
            assertTrue(client.page("c_1").ok());
            assertTrue(client.page("c_1", 10).ok());
            client.cancelJob("j-4");
        }
    }

    @Test
    void closedClientRejectsCalls() {
        NurtClient client = NurtClient.create(options());
        client.close();
        client.close();
        NurtCallException error = assertThrows(NurtCallException.class, () -> client.run(Request.program("x")));
        assertEquals(NurtCallException.Kind.INVALID_ARGUMENT, error.kind());
    }

    @Test
    void missingExecutableIsReportedWithCheckedLocations() {
        Path missing = temp.resolve("nurt");
        NurtCallException error = assertThrows(NurtCallException.class, () -> NurtClient.create(
                new NurtClientOptions().backend(BackendKind.PROCESS).executable(missing)));
        assertEquals(NurtCallException.Kind.NOT_FOUND, error.kind());
        assertTrue(error.getMessage().contains(missing.toString()));
    }

    @Test
    void libraryBackendWithoutItsArtifactIsNotFound() {
        NurtCallException error = assertThrows(NurtCallException.class,
                () -> NurtClient.create(new NurtClientOptions().backend(BackendKind.LIBRARY)));
        assertEquals(NurtCallException.Kind.NOT_FOUND, error.kind());
    }

    @Test
    void automaticBackendFallsBackToTheProcess() {
        try (NurtClient client = NurtClient.create(new NurtClientOptions().command(serverCommand()))) {
            assertEquals(BackendKind.PROCESS, client.backend());
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
