// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.ffm;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Flow;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.Assumptions;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;
import pl.danacogroup.nurt.BackendKind;
import pl.danacogroup.nurt.CallException;
import pl.danacogroup.nurt.Client;
import pl.danacogroup.nurt.ClientOptions;
import pl.danacogroup.nurt.Event;
import pl.danacogroup.nurt.Execution;
import pl.danacogroup.nurt.Request;
import pl.danacogroup.nurt.Response;

/**
 * Client on the library backend.
 *
 * <p>Except for the lookup failure, the tests open an engine of the library
 * named by {@code NURT_LIBRARY} with its state directory in a temporary
 * directory and are skipped when the variable is not set.
 */
class LibraryClientTest {
    private static final Duration WAIT = Duration.ofSeconds(30);
    private static final int THREADS = 8;
    private static final int CALLS_PER_THREAD = 25;

    @TempDir
    Path state;

    private static Path library() {
        String path = System.getenv("NURT_LIBRARY");
        Assumptions.assumeTrue(path != null && !path.isEmpty(), "NURT_LIBRARY is not set");
        return Path.of(path).toAbsolutePath();
    }

    private ClientOptions options(BackendKind backend) {
        return new ClientOptions()
                .backend(backend)
                .library(library())
                .config(Map.of("state", Map.of("dir", state.toString())));
    }

    @Test
    void missingLibraryIsReportedWithItsLocation() {
        Path missing = state.resolve("no-such-library");
        CallException error = assertThrows(CallException.class,
                () -> Client.create(new ClientOptions().backend(BackendKind.LIBRARY).library(missing)));
        assertEquals(CallException.Kind.NOT_FOUND, error.kind());
        assertTrue(error.getMessage().contains(missing.toString()), error.getMessage());
    }

    @Test
    void automaticSelectionPrefersACompatibleLibrary() {
        try (Client nurt = Client.create(options(BackendKind.AUTO))) {
            assertEquals(BackendKind.LIBRARY, nurt.backend());
        }
    }

    @Test
    void infoAndSchemaNeedNoProgram() {
        try (Client nurt = Client.create(options(BackendKind.LIBRARY))) {
            assertFalse(nurt.info().isEmpty());
            assertFalse(nurt.schema("request").isEmpty());
        }
    }

    @Test
    void unknownSchemaIsAnInvalidArgument() {
        try (Client nurt = Client.create(options(BackendKind.LIBRARY))) {
            CallException error = assertThrows(CallException.class, () -> nurt.schema("no-such-schema"));
            assertEquals(CallException.Kind.INVALID_ARGUMENT, error.kind());
        }
    }

    @Test
    void jobCancellationIsRejectedByTheLibraryBackend() {
        try (Client nurt = Client.create(options(BackendKind.LIBRARY))) {
            CallException error = assertThrows(CallException.class, () -> nurt.cancelJob("job-1"));
            assertEquals(CallException.Kind.INVALID_ARGUMENT, error.kind());
        }
    }

    @Test
    void runReturnsAResponse() {
        try (Client nurt = Client.create(options(BackendKind.LIBRARY))) {
            Response response = nurt.run(Request.program("help"));
            assertNotNull(response.report());
        }
    }

    @Test
    void runWithinARequestTimeoutGoesThroughAnExecution() {
        try (Client nurt = Client.create(options(BackendKind.LIBRARY).requestTimeout(WAIT))) {
            assertNotNull(nurt.run(Request.program("help")).report());
        }
    }

    @Test
    void startedExecutionCompletesItsEventStream() throws Exception {
        try (Client nurt = Client.create(options(BackendKind.LIBRARY))) {
            Execution execution = nurt.start(Request.program("help"));
            CountDownLatch done = new CountDownLatch(1);
            AtomicReference<Throwable> failure = new AtomicReference<>();
            List<Event> received = new ArrayList<>();
            execution.events().subscribe(new Flow.Subscriber<>() {
                @Override
                public void onSubscribe(Flow.Subscription subscription) {
                    subscription.request(Long.MAX_VALUE);
                }

                @Override
                public void onNext(Event item) {
                    synchronized (received) {
                        received.add(item);
                    }
                }

                @Override
                public void onError(Throwable throwable) {
                    failure.set(throwable);
                    done.countDown();
                }

                @Override
                public void onComplete() {
                    done.countDown();
                }
            });
            assertNotNull(execution.await(WAIT).report());
            assertTrue(done.await(WAIT.toMillis(), TimeUnit.MILLISECONDS), "event stream completes");
            assertNull(failure.get());
            synchronized (received) {
                for (int i = 1; i < received.size(); i++) {
                    assertTrue(received.get(i - 1).seq().orElse(0) < received.get(i).seq().orElse(Long.MAX_VALUE),
                            "events arrive in increasing seq");
                }
            }
        }
    }

    @Test
    void cancelledExecutionStillReturnsAResponse() {
        try (Client nurt = Client.create(options(BackendKind.LIBRARY))) {
            Execution execution = nurt.start(Request.program("help"));
            execution.cancel();
            execution.cancel();
            assertNotNull(execution.await(WAIT).report());
        }
    }

    @Test
    void callsFromManyThreadsShareOneEngine() throws Exception {
        try (Client nurt = Client.create(options(BackendKind.LIBRARY))) {
            ExecutorService pool = Executors.newFixedThreadPool(THREADS);
            try {
                List<Future<Integer>> results = new ArrayList<>();
                for (int t = 0; t < THREADS; t++) {
                    results.add(pool.submit(() -> {
                        int completed = 0;
                        for (int i = 0; i < CALLS_PER_THREAD; i++) {
                            assertFalse(nurt.info().isEmpty());
                            assertFalse(nurt.schema("report").isEmpty());
                            completed++;
                        }
                        return completed;
                    }));
                }
                for (Future<Integer> result : results) {
                    assertEquals(CALLS_PER_THREAD, result.get(WAIT.toMillis(), TimeUnit.MILLISECONDS));
                }
            } finally {
                pool.shutdownNow();
            }
        }
    }

    @Test
    void closeIsIdempotentAndLaterCallsFail() {
        Client nurt = Client.create(options(BackendKind.LIBRARY));
        nurt.close();
        nurt.close();
        CallException error = assertThrows(CallException.class, nurt::info);
        assertEquals(CallException.Kind.DISCONNECTED, error.kind());
    }

    @Test
    void closeCancelsRunningExecutions() {
        Client nurt = Client.create(options(BackendKind.LIBRARY));
        Execution execution = nurt.start(Request.program("help"));
        nurt.close();
        assertTrue(execution.isDone(), "close waits until the execution is released");
    }
}
