// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/main/java/pl/danacogroup/nurt/NurtClient.java
// Responsibility:   Source file bindings/java/nurt/src/main/java/pl/danacogroup/nurt/NurtClient.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Optional;
import java.util.ServiceLoader;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.Flow;
import java.util.concurrent.ForkJoinPool;
import java.util.concurrent.SubmissionPublisher;
import pl.danacogroup.nurt.internal.EventStream;
import pl.danacogroup.nurt.internal.Json;
import pl.danacogroup.nurt.internal.ProcessBackend;
import pl.danacogroup.nurt.spi.Backend;
import pl.danacogroup.nurt.spi.BackendProvider;
import pl.danacogroup.nurt.spi.BackendSettings;

/**
 * Client of Danaco Nurt.
 *
 * <p>The client is thread-safe: requests of several threads are in flight at
 * the same time. A language error is part of a successful {@link NurtResponse};
 * a failure of the call itself is a {@link NurtCallException} with the status
 * code of the C interface.
 *
 * <pre>{@code
 * try (NurtClient nurt = NurtClient.create()) {
 *     NurtResponse response = nurt.run(Request.program("find \"/home/user/src\" glob: \"*.go\""));
 *     response.error().ifPresentOrElse(
 *             error -> System.err.println(error + " fix: " + error.fix()),
 *             () -> System.out.println(response.text().orElse("")));
 * }
 * }</pre>
 */
public final class NurtClient implements AutoCloseable {
    private static final String LIBRARY_PROVIDER = "pl.danacogroup.nurt.ffm.LibraryBackendProvider";

    private final Backend backend;
    private final BackendKind kind;
    private final SubmissionPublisher<NurtEvent> events;

    private NurtClient(Backend backend, BackendKind kind, SubmissionPublisher<NurtEvent> events) {
        this.backend = backend;
        this.kind = kind;
        this.events = events;
    }

    /**
     * Client with the default options.
     *
     * @return the client
     * @throws NurtCallException when no backend can be opened
     */
    public static NurtClient create() {
        return create(new NurtClientOptions());
    }

    /**
     * Client with the given options.
     *
     * @param options the options
     * @return the client
     * @throws NurtCallException when the requested backend cannot be opened, for
     *     example {@link NurtCallException.Kind#NOT_FOUND} with the checked
     *     locations of the executable
     */
    public static NurtClient create(NurtClientOptions options) {
        SubmissionPublisher<NurtEvent> events = new SubmissionPublisher<>();
        BackendSettings settings = new BackendSettings(
                options.executable(),
                options.command(),
                options.environment(),
                options.library(),
                options.config(),
                options.session(),
                options.language() == null ? null : options.language().wire(),
                options.requestTimeout(),
                value -> events.offer(new NurtEvent(value), null));
        try {
            return switch (options.backend()) {
                case PROCESS -> new NurtClient(ProcessBackend.open(settings), BackendKind.PROCESS, events);
                case LIBRARY -> new NurtClient(libraryProvider()
                        .orElseThrow(() -> new NurtCallException(NurtCallException.Kind.NOT_FOUND,
                                "library backend needs the artifact pl.danaco-group:nurt-ffm and Java 22 or later"))
                        .open(settings), BackendKind.LIBRARY, events);
                case AUTO -> {
                    Optional<BackendProvider> library = libraryProvider()
                            .filter(provider -> provider.isAvailable(settings));
                    yield library.isPresent()
                            ? new NurtClient(library.get().open(settings), BackendKind.LIBRARY, events)
                            : new NurtClient(ProcessBackend.open(settings), BackendKind.PROCESS, events);
                }
            };
        } catch (RuntimeException e) {
            events.close();
            throw e;
        }
    }

    /**
     * Provider of the library backend: a registered service, or on the class
     * path the provider class of the artifact {@code nurt-ffm}.
     */
    private static Optional<BackendProvider> libraryProvider() {
        for (BackendProvider provider : ServiceLoader.load(BackendProvider.class, NurtClient.class.getClassLoader())) {
            if (provider.kind() == BackendKind.LIBRARY) {
                return Optional.of(provider);
            }
        }
        try {
            Class<?> type = Class.forName(LIBRARY_PROVIDER, true, NurtClient.class.getClassLoader());
            Object provider = type.getConstructor().newInstance();
            return provider instanceof BackendProvider library ? Optional.of(library) : Optional.empty();
        } catch (ReflectiveOperationException | LinkageError e) {
            return Optional.empty();
        }
    }

    /**
     * Backend in use.
     *
     * @return {@link BackendKind#PROCESS} or {@link BackendKind#LIBRARY}
     */
    public BackendKind backend() {
        return kind;
    }

    /**
     * Runs a request and waits for its response.
     *
     * @param request the request
     * @return the response; a language error is read with {@link NurtResponse#error()}
     * @throws NurtCallException when the call fails
     */
    public NurtResponse run(Request request) {
        return NurtResponse.of(backend.call("run", request.toMap()));
    }

    /**
     * Runs a request without blocking.
     *
     * @param request the request
     * @return the future response; it fails with a {@link NurtCallException}
     */
    public CompletableFuture<NurtResponse> runAsync(Request request) {
        try {
            return backend.start(request.toMap(), value -> { }).result().thenApply(NurtResponse::of);
        } catch (NurtCallException e) {
            return CompletableFuture.failedFuture(e);
        }
    }

    /**
     * Starts a request and returns at once.
     *
     * @param request the request
     * @return the execution with its events and response
     * @throws NurtCallException when the request cannot be sent
     */
    public Execution start(Request request) {
        EventStream<NurtEvent> stream = new EventStream<>(ForkJoinPool.commonPool());
        return new Execution(backend.start(request.toMap(), value -> stream.publish(new NurtEvent(value))), stream);
    }

    /**
     * Binds a request and classifies its effect without running it
     * ({@code nurt-check/1}).
     *
     * @param request the request
     * @return the classification
     * @throws NurtCallException when the call fails
     */
    public Map<String, Object> check(Request request) {
        return object("check", backend.call("check", request.toMap()));
    }

    /**
     * Parses program text into a syntax tree {@code nurt-ast/1} with diagnostics.
     *
     * @param program the program
     * @param variant its notation variant
     * @return the tree and the diagnostics
     * @throws NurtCallException when the call fails
     */
    public Map<String, Object> parse(String program, Request.Variant variant) {
        return object("parse", backend.call("parse", params("program", program, "variant", variant.wire())));
    }

    /**
     * Prints a syntax tree as the canonical text of a variant.
     *
     * @param ast the tree {@code nurt-ast/1}
     * @param variant the notation variant
     * @return the program text
     * @throws NurtCallException when the call fails
     */
    public String print(Map<String, Object> ast, Request.Variant variant) {
        return text("print", backend.call("print", params("ast", ast, "variant", variant.wire())));
    }

    /**
     * Corrects the written form of a program without running it.
     *
     * @param program the program
     * @param variant its notation variant
     * @return the corrected program and the corrections
     * @throws NurtCallException when the call fails
     */
    public Map<String, Object> correct(String program, Request.Variant variant) {
        return object("correct", backend.call("correct", params("program", program, "variant", variant.wire())));
    }

    /**
     * Reads a further page with the limit of the original request.
     *
     * @param cursor the cursor
     * @return the page
     * @throws NurtCallException when the call fails
     */
    public NurtResponse page(String cursor) {
        return NurtResponse.of(backend.call("page", params("cursor", cursor)));
    }

    /**
     * Reads a further page.
     *
     * @param cursor the cursor
     * @param limit records per page
     * @return the page
     * @throws NurtCallException when the call fails
     */
    public NurtResponse page(String cursor, int limit) {
        return NurtResponse.of(backend.call("page", params("cursor", cursor, "limit", (long) limit)));
    }

    /**
     * Core description of the language for tool integrations.
     *
     * @return the description
     * @throws NurtCallException when the call fails
     */
    public String describe() {
        return text("describe", backend.call("describe", params("subject", "core")));
    }

    /**
     * Description of one group of commands.
     *
     * @param group the group, for example {@code documents}
     * @param language language of the description, or {@code null} for the default
     * @return the description
     * @throws NurtCallException when the call fails
     */
    public String describe(String group, Request.Language language) {
        Map<String, Object> params = params("subject", group);
        if (language != null) {
            params.put("language", language.wire());
        }
        return text("describe", backend.call("describe", params));
    }

    /**
     * Index of help topics.
     *
     * @return the index
     * @throws NurtCallException when the call fails
     */
    public String help() {
        return text("help", backend.call("help", params()));
    }

    /**
     * Built-in help on a topic.
     *
     * @param topic the topic
     * @return the help text
     * @throws NurtCallException when the call fails
     */
    public String help(String topic) {
        return text("help", backend.call("help", params("topic", topic)));
    }

    /**
     * Whole command catalog {@code nurt-catalog/1}.
     *
     * @return the catalog
     * @throws NurtCallException when the call fails
     */
    public Map<String, Object> catalog() {
        return object("catalog", backend.call("catalog", params()));
    }

    /**
     * Part of the command catalog.
     *
     * @param what selection of a group or a command
     * @return the selected part
     * @throws NurtCallException when the call fails
     */
    public Map<String, Object> catalog(Map<String, Object> what) {
        return object("catalog", backend.call("catalog", new LinkedHashMap<>(what)));
    }

    /**
     * Published JSON schema by name, for example {@code request} or {@code report}.
     *
     * @param which the name
     * @return the schema
     * @throws NurtCallException when the call fails; an unknown name is
     *     {@link NurtCallException.Kind#INVALID_ARGUMENT}
     */
    public Map<String, Object> schema(String which) {
        return object("schema", backend.call("schema", params("name", which)));
    }

    /**
     * Journal history.
     *
     * @return the history
     * @throws NurtCallException when the call fails
     */
    public Map<String, Object> history() {
        return object("history", backend.call("history", params()));
    }

    /**
     * Journal history selected by the parameters of the command {@code history}.
     *
     * @param query the parameters
     * @return the history
     * @throws NurtCallException when the call fails
     */
    public Map<String, Object> history(Map<String, Object> query) {
        return object("history", backend.call("history", new LinkedHashMap<>(query)));
    }

    /**
     * Free space and journal usage of every volume.
     *
     * @return the state of the volumes
     * @throws NurtCallException when the call fails
     */
    public Map<String, Object> space() {
        return object("space", backend.call("space", params()));
    }

    /**
     * Free space and journal usage of the volume holding a path.
     *
     * @param volume a path on the volume
     * @return the state of the volume
     * @throws NurtCallException when the call fails
     */
    public Map<String, Object> space(String volume) {
        return object("space", backend.call("space", params("volume", volume)));
    }

    /**
     * Version, interface and format versions, catalog revision and features.
     *
     * @return the information
     * @throws NurtCallException when the call fails
     */
    public Map<String, Object> info() {
        return object("info", backend.call("info", params()));
    }

    /**
     * Cancels a background job.
     *
     * @param jobId identifier of the job
     * @throws NurtCallException when the call fails
     */
    public void cancelJob(String jobId) {
        backend.call("cancel", params("job_id", jobId));
    }

    /**
     * Events that belong to no execution started with {@link #start(Request)}:
     * background jobs, journal changes, free space, schedules and log
     * messages. A subscriber receives the events published after it
     * subscribes; events beyond the buffer of a slow subscriber are dropped.
     *
     * @return the publisher
     */
    public Flow.Publisher<NurtEvent> events() {
        return events;
    }

    /** Shuts the server down or releases the library engine. */
    @Override
    public void close() {
        try {
            backend.close();
        } finally {
            events.close();
        }
    }

    private static Map<String, Object> params(Object... namesAndValues) {
        Map<String, Object> params = new LinkedHashMap<>();
        for (int i = 0; i + 1 < namesAndValues.length; i += 2) {
            params.put((String) namesAndValues[i], namesAndValues[i + 1]);
        }
        return params;
    }

    private static Map<String, Object> object(String method, Object result) {
        Map<String, Object> object = Json.object(result);
        if (object == null) {
            throw new NurtCallException(NurtCallException.Kind.PROTOCOL, "result of `" + method + "` is not an object");
        }
        return object;
    }

    private static String text(String method, Object result) {
        if (result instanceof String text) {
            return text;
        }
        if (Json.object(result) != null
                && (Json.string(result, "text") != null || Json.string(result, "description") != null)) {
            String text = Json.string(result, "text");
            return text != null ? text : Json.string(result, "description");
        }
        throw new NurtCallException(NurtCallException.Kind.PROTOCOL, "result of `" + method + "` is neither a string nor an object with `text` or `description`");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
