// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt-ffm/src/main/java/pl/danacogroup/nurt/ffm/LibraryBackend.java
// Responsibility:   Source file bindings/java/nurt-ffm/src/main/java/pl/danacogroup/nurt/ffm/LibraryBackend.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt.ffm;

import static java.lang.foreign.ValueLayout.ADDRESS;

import java.lang.foreign.Arena;
import java.lang.foreign.Linker;
import java.lang.foreign.MemorySegment;
import java.lang.invoke.MethodHandle;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.MethodType;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.function.Consumer;
import pl.danacogroup.nurt.NurtCallException;
import pl.danacogroup.nurt.CallStatus;
import pl.danacogroup.nurt.internal.Json;
import pl.danacogroup.nurt.internal.JsonException;
import pl.danacogroup.nurt.spi.Backend;
import pl.danacogroup.nurt.spi.BackendSettings;
import pl.danacogroup.nurt.spi.StartedCall;

/**
 * Backend that calls an engine of {@code libnurt} in the process.
 *
 * <p>Each synchronous call allocates its arguments and output buffers in a
 * confined arena that is closed when the call returns; every output buffer
 * is released with {@code nurt_buf_free}. An execution started with
 * {@link #start} owns a shared arena holding the upcall stub of its event
 * callback; a dedicated thread waits for the execution, releases its handle
 * with {@code nurt_exec_free} and then closes the arena.
 *
 * <p>{@link #close} cancels the running executions, waits until every call
 * in progress has returned and then closes the engine, so the engine handle
 * is never used after {@code nurt_engine_close}.
 */
final class LibraryBackend implements Backend {
    /** Notation variants in the order of {@code nurt_variant}, starting at 1. */
    private static final List<String> VARIANTS = List.of("a", "b", "c");

    /** Subject of {@code describe} that selects the core description. */
    private static final String CORE_SUBJECT = "core";

    private static final MethodHandle DELIVER;

    static {
        try {
            DELIVER = MethodHandles.lookup().findStatic(LibraryBackend.class, "deliver",
                    MethodType.methodType(void.class, Consumer.class, MemorySegment.class, MemorySegment.class));
        } catch (ReflectiveOperationException e) {
            throw new ExceptionInInitializerError(e);
        }
    }

    private final Library library;
    private final MemorySegment engine;
    private final String session;
    private final String language;
    private final Duration requestTimeout;
    private final Set<Started> executions = ConcurrentHashMap.newKeySet();
    private int active;
    private boolean closed;

    private LibraryBackend(Library library, MemorySegment engine, BackendSettings settings) {
        this.library = library;
        this.engine = engine;
        this.session = settings.session();
        this.language = settings.language();
        this.requestTimeout = settings.requestTimeout();
    }

    /**
     * Opens an engine with the configuration of the settings.
     *
     * @param library the loaded library; its interface version is checked
     * @param settings the settings; {@code config} is passed to
     *     {@code nurt_engine_open}
     * @return the backend
     * @throws NurtCallException when the engine cannot be opened
     */
    static LibraryBackend open(Library library, BackendSettings settings) {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment config = settings.config() == null
                    ? MemorySegment.NULL
                    : utf8(arena, Json.write(settings.config()));
            MemorySegment out = arena.allocate(ADDRESS);
            int status = Library.invokeInt(library.engineOpen, config, out);
            library.verify(status, MemorySegment.NULL, "nurt_engine_open");
            return new LibraryBackend(library, out.get(ADDRESS, 0), settings);
        }
    }

    @Override
    public Object call(String method, Map<String, Object> params) {
        if ("run".equals(method) && requestTimeout != null) {
            return runWithin(params, requestTimeout);
        }
        enter();
        try (Arena arena = Arena.ofConfined()) {
            return dispatch(arena, method, params);
        } finally {
            leave();
        }
    }

    @Override
    public StartedCall start(Map<String, Object> request, Consumer<Map<String, Object>> events) {
        enter();
        Arena arena = Arena.ofShared();
        Started started;
        try {
            MemorySegment callback = events == null
                    ? MemorySegment.NULL
                    : Linker.nativeLinker().upcallStub(
                            MethodHandles.insertArguments(DELIVER, 0, events), Library.EVENT_CALLBACK, arena);
            MemorySegment out = arena.allocate(ADDRESS);
            int status = Library.invokeInt(library.execStart, engine,
                    utf8(arena, Json.write(withDefaults(request))), callback, MemorySegment.NULL, out);
            library.verify(status, engine, "nurt_exec_start");
            started = new Started(out.get(ADDRESS, 0), arena);
        } catch (RuntimeException e) {
            arena.close();
            leave();
            throw e;
        }
        executions.add(started);
        if (isClosed()) {
            started.cancel();
        }
        Thread waiter = new Thread(() -> await(started), "nurt-exec-wait");
        waiter.setDaemon(true);
        waiter.start();
        return started;
    }

    @Override
    public void close() {
        synchronized (this) {
            if (closed) {
                return;
            }
            closed = true;
        }
        for (Started execution : List.copyOf(executions)) {
            try {
                execution.cancel();
            } catch (NurtCallException e) {
                continue;
            }
        }
        boolean interrupted = false;
        synchronized (this) {
            while (active > 0) {
                try {
                    wait();
                } catch (InterruptedException e) {
                    interrupted = true;
                }
            }
        }
        Library.invokeVoid(library.engineClose, engine);
        if (interrupted) {
            Thread.currentThread().interrupt();
        }
    }

    private synchronized boolean isClosed() {
        return closed;
    }

    private synchronized void enter() {
        if (closed) {
            throw new NurtCallException(NurtCallException.Kind.INVALID_ARGUMENT, "client is closed");
        }
        active++;
    }

    private synchronized void leave() {
        active--;
        if (active == 0) {
            notifyAll();
        }
    }

    private Object runWithin(Map<String, Object> request, Duration timeout) {
        StartedCall started = start(request, null);
        try {
            return started.result().get(timeout.toMillis(), TimeUnit.MILLISECONDS);
        } catch (TimeoutException e) {
            started.cancel();
            throw new NurtCallException(NurtCallException.Kind.TIMEOUT,
                    "no response to `run` within " + timeout.toMillis() + " ms; raise the request timeout", e);
        } catch (ExecutionException e) {
            if (e.getCause() instanceof NurtCallException call) {
                throw call;
            }
            throw new NurtCallException(NurtCallException.Kind.INTERNAL, "request failed: " + e.getCause(), e);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            started.cancel();
            throw new NurtCallException(NurtCallException.Kind.CANCELLED, "waiting for `run` was interrupted", e);
        }
    }

    private Object dispatch(Arena arena, String method, Map<String, Object> params) {
        return switch (method) {
            case "run" -> json(callWithBuffer(arena, library.runJson, "nurt_run_json",
                    utf8(arena, Json.write(withDefaults(params)))), "nurt_run_json");
            case "check" -> check(arena, params);
            case "parse" -> parse(arena, params);
            case "print" -> callWithBuffer(arena, library.print, "nurt_print",
                    utf8(arena, Json.write(required(params, "ast"))), variant(params));
            case "correct" -> json(callWithBuffer(arena, library.correct, "nurt_correct",
                    text(arena, params, "program"), variant(params)), "nurt_correct");
            case "page" -> json(callWithBuffer(arena, library.page, "nurt_page",
                    text(arena, params, "cursor"), limit(params)), "nurt_page");
            case "describe" -> callWithBuffer(arena, library.describe, "nurt_describe",
                    utf8(arena, Json.write(describeSelection(params))));
            case "help" -> callWithBuffer(arena, library.help, "nurt_help", optionalText(arena, params, "topic"));
            case "catalog" -> json(callWithBuffer(arena, library.catalog, "nurt_catalog",
                    selection(arena, params)), "nurt_catalog");
            case "history" -> json(callWithBuffer(arena, library.history, "nurt_history",
                    selection(arena, params)), "nurt_history");
            case "space" -> json(callWithBuffer(arena, library.space, "nurt_space",
                    optionalText(arena, params, "volume")), "nurt_space");
            case "schema" -> json(callWithoutEngine(arena, library.schema, "nurt_schema",
                    text(arena, params, "name")), "nurt_schema");
            case "info" -> json(callWithoutEngine(arena, library.info, "nurt_info"), "nurt_info");
            case "cancel" -> throw new NurtCallException(NurtCallException.Kind.INVALID_ARGUMENT,
                    "the library backend cannot cancel background jobs; use the process backend");
            default -> throw new NurtCallException(NurtCallException.Kind.INVALID_ARGUMENT,
                    "unknown method `" + method + "`");
        };
    }

    private Object check(Arena arena, Map<String, Object> params) {
        MemorySegment bound = arena.allocate(Library.BUF);
        MemorySegment diagnostics = arena.allocate(Library.BUF);
        try {
            int status = Library.invokeInt(library.check, engine,
                    utf8(arena, Json.write(withDefaults(params))), bound, diagnostics);
            library.verify(status, engine, "nurt_check");
            Object result = json(library.take(bound), "nurt_check");
            String diagnosticsText = library.take(diagnostics);
            Map<String, Object> object = Json.object(result);
            if (object != null && !diagnosticsText.isEmpty() && !object.containsKey("diagnostics")) {
                Map<String, Object> merged = new LinkedHashMap<>(object);
                merged.put("diagnostics", json(diagnosticsText, "nurt_check"));
                return merged;
            }
            return result;
        } finally {
            release(bound, diagnostics);
        }
    }

    private Object parse(Arena arena, Map<String, Object> params) {
        MemorySegment ast = arena.allocate(Library.BUF);
        MemorySegment diagnostics = arena.allocate(Library.BUF);
        try {
            int status = Library.invokeInt(library.parse, engine,
                    text(arena, params, "program"), variant(params), ast, diagnostics);
            library.verify(status, engine, "nurt_parse");
            String astText = library.take(ast);
            String diagnosticsText = library.take(diagnostics);
            Map<String, Object> result = new LinkedHashMap<>();
            result.put("ast", astText.isEmpty() ? null : json(astText, "nurt_parse"));
            result.put("diagnostics", diagnosticsText.isEmpty() ? new ArrayList<>() : json(diagnosticsText, "nurt_parse"));
            return result;
        } finally {
            release(ast, diagnostics);
        }
    }

    /** Calls a function {@code (engine, arguments..., nurt_buf *out)} and returns the text of the buffer. */
    private String callWithBuffer(Arena arena, MethodHandle function, String name, Object... arguments) {
        Object[] all = new Object[arguments.length + 2];
        all[0] = engine;
        System.arraycopy(arguments, 0, all, 1, arguments.length);
        return invokeWithBuffer(arena, function, name, all, engine);
    }

    /** Calls a function {@code (arguments..., nurt_buf *out)} that needs no engine. */
    private String callWithoutEngine(Arena arena, MethodHandle function, String name, Object... arguments) {
        Object[] all = new Object[arguments.length + 1];
        System.arraycopy(arguments, 0, all, 0, arguments.length);
        return invokeWithBuffer(arena, function, name, all, MemorySegment.NULL);
    }

    private String invokeWithBuffer(Arena arena, MethodHandle function, String name, Object[] arguments,
            MemorySegment errorEngine) {
        MemorySegment buffer = arena.allocate(Library.BUF);
        arguments[arguments.length - 1] = buffer;
        try {
            int status = Library.invokeInt(function, arguments);
            library.verify(status, errorEngine, name);
            return library.take(buffer);
        } finally {
            release(buffer);
        }
    }

    private void release(MemorySegment... buffers) {
        for (MemorySegment buffer : buffers) {
            Library.invokeVoid(library.bufFree, buffer);
        }
    }

    private void await(Started execution) {
        Object result = null;
        Throwable failure = null;
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment buffer = arena.allocate(Library.BUF);
            try {
                int status = Library.invokeInt(library.execWait, execution.handle, Library.WAIT_FOREVER, buffer);
                if (status != CallStatus.CANCELLED.code()) {
                    library.verify(status, engine, "nurt_exec_wait");
                }
                result = json(library.take(buffer), "nurt_exec_wait");
            } finally {
                release(buffer);
            }
        } catch (NurtCallException e) {
            failure = e;
        } catch (Throwable e) {
            failure = new NurtCallException(NurtCallException.Kind.INTERNAL, "waiting for the execution failed: " + e, e);
        } finally {
            try {
                execution.free();
            } finally {
                if (failure != null) {
                    execution.future.completeExceptionally(failure);
                } else {
                    execution.future.complete(result);
                }
                executions.remove(execution);
                leave();
            }
        }
    }

    /**
     * Event callback of an execution, called by a thread of the library.
     *
     * <p>A throwable escaping an upcall ends the Java virtual machine, so every
     * failure of the receiver is contained here and the event is dropped.
     */
    private static void deliver(Consumer<Map<String, Object>> receiver, MemorySegment user, MemorySegment eventJson) {
        try {
            if (eventJson.equals(MemorySegment.NULL)) {
                return;
            }
            Map<String, Object> event = Json.object(Json.parse(eventJson.reinterpret(Long.MAX_VALUE).getString(0)));
            if (event != null) {
                receiver.accept(event);
            }
        } catch (Throwable e) {
            return;
        }
    }

    private Map<String, Object> withDefaults(Map<String, Object> request) {
        Map<String, Object> envelope = new LinkedHashMap<>(request);
        Map<String, Object> options = Json.object(envelope.get("options"));
        Map<String, Object> merged = options == null ? new LinkedHashMap<>() : new LinkedHashMap<>(options);
        if (session != null) {
            merged.putIfAbsent("session", session);
        }
        if (language != null) {
            merged.putIfAbsent("language", language);
        }
        if (!merged.isEmpty()) {
            envelope.put("options", merged);
        }
        return envelope;
    }

    private Map<String, Object> describeSelection(Map<String, Object> params) {
        Map<String, Object> selection = new LinkedHashMap<>();
        Object subject = params.getOrDefault("subject", CORE_SUBJECT);
        if (CORE_SUBJECT.equals(subject)) {
            selection.put("layer", CORE_SUBJECT);
        } else {
            selection.put("layer", "group");
            selection.put("name", subject);
        }
        if (params.get("language") != null) {
            selection.put("language", params.get("language"));
        }
        if (language != null) {
            selection.putIfAbsent("language", language);
        }
        return selection;
    }

    private static MemorySegment selection(Arena arena, Map<String, Object> params) {
        return params.isEmpty() ? MemorySegment.NULL : utf8(arena, Json.write(params));
    }

    static Object required(Map<String, Object> params, String name) {
        Object value = params.get(name);
        if (value == null) {
            throw new NurtCallException(NurtCallException.Kind.INVALID_ARGUMENT, "parameter `" + name + "` is required");
        }
        return value;
    }

    /**
     * Text as UTF-8 followed by a zero byte, which the library reads as a C
     * string. A text that holds U+0000 would be cut at that character and a
     * text with an unpaired surrogate would be changed, so both are refused.
     *
     * @param arena the arena that owns the copy
     * @param value the text
     * @return the copy
     * @throws NurtCallException of kind {@link NurtCallException.Kind#INVALID_ARGUMENT}
     *     for a text that cannot be passed unchanged
     */
    static MemorySegment utf8(Arena arena, String value) {
        if (value.indexOf('\0') >= 0) {
            throw new NurtCallException(NurtCallException.Kind.INVALID_ARGUMENT,
                    "text must not contain the character U+0000");
        }
        if (!StandardCharsets.UTF_8.newEncoder().canEncode(value)) {
            throw new NurtCallException(NurtCallException.Kind.INVALID_ARGUMENT,
                    "text is not valid Unicode: it holds an unpaired surrogate");
        }
        return arena.allocateFrom(value);
    }

    static MemorySegment text(Arena arena, Map<String, Object> params, String name) {
        if (required(params, name) instanceof String value) {
            return utf8(arena, value);
        }
        throw new NurtCallException(NurtCallException.Kind.INVALID_ARGUMENT, "parameter `" + name + "` must be a string");
    }

    static MemorySegment optionalText(Arena arena, Map<String, Object> params, String name) {
        return params.get(name) == null ? MemorySegment.NULL : text(arena, params, name);
    }

    static int variant(Map<String, Object> params) {
        Object value = params.get("variant");
        int index = value == null ? 0 : VARIANTS.indexOf(value);
        if (index < 0) {
            throw new NurtCallException(NurtCallException.Kind.INVALID_ARGUMENT,
                    "parameter `variant` must be one of a, b, c; got `" + value + "`");
        }
        return index + 1;
    }

    static int limit(Map<String, Object> params) {
        Object value = params.get("limit");
        if (value == null) {
            return 0;
        }
        if (value instanceof Number number && number.longValue() > 0 && number.longValue() <= Integer.MAX_VALUE) {
            return number.intValue();
        }
        throw new NurtCallException(NurtCallException.Kind.INVALID_ARGUMENT,
                "parameter `limit` must be a positive integer; got `" + value + "`");
    }

    private static Object json(String text, String function) {
        if (text.isEmpty()) {
            throw new NurtCallException(NurtCallException.Kind.PROTOCOL, function + " returned an empty result");
        }
        try {
            return Json.parse(text);
        } catch (JsonException e) {
            throw new NurtCallException(NurtCallException.Kind.PROTOCOL,
                    function + " returned text that is not valid JSON: " + e.getMessage(), e);
        }
    }

    /** Execution started with {@code nurt_exec_start}. */
    private final class Started implements StartedCall {
        final MemorySegment handle;
        final CompletableFuture<Object> future = new CompletableFuture<>();
        private final Arena arena;
        private boolean freed;

        Started(MemorySegment handle, Arena arena) {
            this.handle = handle;
            this.arena = arena;
        }

        @Override
        public CompletableFuture<Object> result() {
            return future;
        }

        @Override
        public synchronized void cancel() {
            if (freed) {
                return;
            }
            library.verify(Library.invokeInt(library.execCancel, handle), engine, "nurt_exec_cancel");
        }

        /** Releases the handle and then the upcall stub, after which no callback can arrive. */
        synchronized void free() {
            if (freed) {
                return;
            }
            freed = true;
            try {
                Library.invokeVoid(library.execFree, handle);
            } finally {
                arena.close();
            }
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
