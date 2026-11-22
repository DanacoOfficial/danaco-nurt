// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.internal;

import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.Writer;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Consumer;
import pl.danacogroup.nurt.CallException;
import pl.danacogroup.nurt.CallStatus;
import pl.danacogroup.nurt.spi.Backend;
import pl.danacogroup.nurt.spi.BackendSettings;
import pl.danacogroup.nurt.spi.StartedCall;

/**
 * Backend that runs {@code nurt serve --stdio} as a child process and speaks
 * {@code nurt-rpc/1}: JSON-RPC 2.0 with one message per line.
 *
 * <p>Responses are matched to requests by {@code id}, so requests of several
 * threads are in flight at the same time. When the process ends, the
 * requests in flight fail with {@link CallException.Kind#DISCONNECTED} and
 * the next request starts a new process.
 */
public final class ProcessBackend implements Backend {
    /** Name and major version of the protocol. */
    public static final String PROTOCOL = "nurt-rpc/1";

    /** Largest message accepted from the server, in characters. */
    static final int MAX_MESSAGE_CHARS = 64 * 1024 * 1024;

    /** Application error codes are {@code APPLICATION_ERROR_BASE - status}. */
    static final long APPLICATION_ERROR_BASE = -32000;

    static final long APPLICATION_ERROR_LOWEST = -32099;
    static final long PARSE_ERROR = -32700;
    static final long INVALID_REQUEST = -32600;
    static final long METHOD_NOT_FOUND = -32601;
    static final long INVALID_PARAMS = -32602;
    static final long INTERNAL_ERROR = -32603;

    /** Time limit of {@code initialize} without a configured request timeout. */
    static final Duration INITIALIZE_TIMEOUT = Duration.ofSeconds(30);

    /** Time the server is given to end after {@code shutdown}. */
    static final Duration SHUTDOWN_GRACE = Duration.ofSeconds(5);

    private static final String CLIENT_NAME = "pl.danaco-group:nurt";
    private static final String CLIENT_VERSION = "0.1.0";

    private final List<String> command;
    private final Map<String, String> environment;
    private final Map<String, Object> initialize;
    private final Duration requestTimeout;
    private final Consumer<Map<String, Object>> events;
    private Connection connection;
    private boolean closed;

    private ProcessBackend(BackendSettings settings, List<String> command) {
        this.command = command;
        this.environment = settings.environment();
        this.initialize = initializeParams(settings);
        this.requestTimeout = settings.requestTimeout();
        this.events = settings.events();
    }

    /**
     * Starts the server and initializes the connection.
     *
     * @param settings the settings; without {@code command} the executable
     *     is located and started as {@code nurt serve --stdio}
     * @return the backend
     * @throws CallException when the executable is not found, the server
     *     cannot be started or uses another major version of the protocol
     */
    public static ProcessBackend open(BackendSettings settings) {
        List<String> command = settings.command();
        if (command.isEmpty()) {
            command = List.of(Artifacts.locateExecutable(settings.executable()).toString(), "serve", "--stdio");
        }
        ProcessBackend backend = new ProcessBackend(settings, command);
        backend.connection();
        return backend;
    }

    static Map<String, Object> initializeParams(BackendSettings settings) {
        Map<String, Object> params = new LinkedHashMap<>();
        params.put("protocol", PROTOCOL);
        params.put("client", new LinkedHashMap<>(Map.of("name", CLIENT_NAME, "version", CLIENT_VERSION)));
        params.put("capabilities", new LinkedHashMap<>(Map.of("events", Boolean.TRUE)));
        if (settings.language() != null) {
            params.put("language", settings.language());
        }
        if (settings.session() != null) {
            params.put("session", settings.session());
        }
        if (settings.config() != null) {
            params.put("config_overrides", settings.config());
        }
        return params;
    }

    @Override
    public Object call(String method, Map<String, Object> params) {
        Connection current = connection();
        Pending pending = current.send(method, params, null);
        return current.await(pending, method, requestTimeout);
    }

    @Override
    public StartedCall start(Map<String, Object> request, Consumer<Map<String, Object>> events) {
        Connection current = connection();
        Pending pending = current.send("run", request, events);
        return new StartedCall() {
            @Override
            public CompletableFuture<Object> result() {
                return pending.future;
            }

            @Override
            public void cancel() {
                if (!pending.future.isDone()) {
                    current.cancel(pending.id);
                }
            }
        };
    }

    @Override
    public void close() {
        Connection current;
        synchronized (this) {
            if (closed) {
                return;
            }
            closed = true;
            current = connection;
            connection = null;
        }
        if (current != null) {
            current.shutdown(SHUTDOWN_GRACE);
        }
    }

    private synchronized Connection connection() {
        if (closed) {
            throw new CallException(CallException.Kind.DISCONNECTED, "client is closed");
        }
        if (connection != null && connection.isOpen()) {
            return connection;
        }
        if (connection != null) {
            connection.terminate(Duration.ZERO);
            connection = null;
        }
        Connection fresh = Connection.start(command, environment, events);
        try {
            Duration limit = requestTimeout == null ? INITIALIZE_TIMEOUT : requestTimeout;
            Object result = fresh.await(fresh.send("initialize", initialize, null), "initialize", limit);
            String protocol = Json.string(result, "protocol");
            if (!isCompatible(protocol)) {
                throw new CallException(CallException.Kind.INCOMPATIBLE,
                        "nurt serve uses protocol `" + protocol + "`, this client needs `" + PROTOCOL
                                + "`; install a matching version of nurt");
            }
        } catch (CallException e) {
            fresh.terminate(Duration.ZERO);
            throw e;
        }
        connection = fresh;
        return fresh;
    }

    /**
     * Whether the server protocol named in {@code initialize} is compatible.
     *
     * @param protocol the protocol, or {@code null}
     * @return {@code true} for {@code nurt-rpc/1} and its minor versions
     */
    static boolean isCompatible(String protocol) {
        return protocol != null
                && (protocol.equals(PROTOCOL) || protocol.startsWith(PROTOCOL + "."));
    }

    /**
     * Call error for a JSON-RPC error object of the server.
     *
     * @param error the error object
     * @return the exception
     */
    static CallException rpcError(Object error) {
        long code = Json.member(error, "code") instanceof Number number ? number.longValue() : INTERNAL_ERROR;
        String message = Json.string(error, "message");
        if (message == null) {
            message = "error without a message";
        }
        CallException.Kind kind;
        if (code == INVALID_PARAMS) {
            kind = CallException.Kind.INVALID_ARGUMENT;
        } else if (code == INTERNAL_ERROR) {
            kind = CallException.Kind.INTERNAL;
        } else if (code == PARSE_ERROR || code == INVALID_REQUEST || code == METHOD_NOT_FOUND) {
            kind = CallException.Kind.PROTOCOL;
        } else if (code >= APPLICATION_ERROR_LOWEST && code < APPLICATION_ERROR_BASE) {
            kind = CallStatus.fromCode((int) (APPLICATION_ERROR_BASE - code))
                    .map(CallException.Kind::of)
                    .orElse(CallException.Kind.INTERNAL);
        } else {
            kind = CallException.Kind.PROTOCOL;
        }
        return new CallException(kind, "nurt serve: " + message + " (code " + code + ")");
    }

    static Long requestId(Object value) {
        if (value instanceof Long number && number >= 0) {
            return number;
        }
        if (value instanceof String text) {
            try {
                return Long.valueOf(text);
            } catch (NumberFormatException e) {
                return null;
            }
        }
        return null;
    }

    private static final class Pending {
        final long id;
        final CompletableFuture<Object> future = new CompletableFuture<>();
        final Consumer<Map<String, Object>> events;

        Pending(long id, Consumer<Map<String, Object>> events) {
            this.id = id;
            this.events = events;
        }
    }

    /** One server process and its streams. */
    private static final class Connection {
        private final Process process;
        private final Writer writer;
        private final Consumer<Map<String, Object>> events;
        private final Map<Long, Pending> pending = new ConcurrentHashMap<>();
        private final AtomicLong ids = new AtomicLong(1);
        private CallException failure;

        private Connection(Process process, Consumer<Map<String, Object>> events) {
            this.process = process;
            this.events = events;
            this.writer = new BufferedWriter(new OutputStreamWriter(process.getOutputStream(), StandardCharsets.UTF_8));
        }

        static Connection start(List<String> command, Map<String, String> environment,
                Consumer<Map<String, Object>> events) {
            ProcessBuilder builder = new ProcessBuilder(command).redirectError(ProcessBuilder.Redirect.INHERIT);
            builder.environment().putAll(environment);
            Process process;
            try {
                process = builder.start();
            } catch (IOException e) {
                throw new CallException(CallException.Kind.IO,
                        "cannot start nurt serve (" + String.join(" ", command) + "): " + e.getMessage(), e);
            }
            Connection connection = new Connection(process, events);
            Thread reader = new Thread(connection::read, "nurt-serve-reader");
            reader.setDaemon(true);
            reader.start();
            return connection;
        }

        synchronized boolean isOpen() {
            return failure == null;
        }

        Pending send(String method, Map<String, Object> params, Consumer<Map<String, Object>> eventReceiver) {
            Pending entry = new Pending(ids.getAndIncrement(), eventReceiver);
            synchronized (this) {
                if (failure != null) {
                    throw failure;
                }
                pending.put(entry.id, entry);
            }
            try {
                write(entry.id, method, params);
            } catch (CallException e) {
                pending.remove(entry.id);
                throw e;
            }
            return entry;
        }

        private void write(long id, String method, Map<String, Object> params) {
            Map<String, Object> message = new LinkedHashMap<>();
            message.put("jsonrpc", "2.0");
            message.put("id", id);
            message.put("method", method);
            message.put("params", params);
            String line = Json.write(message) + "\n";
            synchronized (writer) {
                try {
                    writer.write(line);
                    writer.flush();
                } catch (IOException e) {
                    CallException error = new CallException(CallException.Kind.DISCONNECTED,
                            "cannot write to nurt serve: " + e.getMessage(), e);
                    fail(error);
                    throw error;
                }
            }
        }

        Object await(Pending entry, String method, Duration timeout) {
            try {
                return timeout == null
                        ? entry.future.get()
                        : entry.future.get(timeout.toMillis(), TimeUnit.MILLISECONDS);
            } catch (TimeoutException e) {
                abandon(entry.id);
                throw new CallException(CallException.Kind.TIMEOUT,
                        "no response to `" + method + "` within " + timeout.toMillis()
                                + " ms; raise the request timeout", e);
            } catch (ExecutionException e) {
                if (e.getCause() instanceof CallException call) {
                    throw call;
                }
                throw new CallException(CallException.Kind.INTERNAL, "request failed: " + e.getCause(), e);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                abandon(entry.id);
                throw new CallException(CallException.Kind.CANCELLED,
                        "waiting for `" + method + "` was interrupted", e);
            }
        }

        void abandon(long id) {
            pending.remove(id);
            cancel(id);
        }

        void cancel(long requestId) {
            Map<String, Object> params = new LinkedHashMap<>();
            params.put("request_id", requestId);
            try {
                write(ids.getAndIncrement(), "cancel", params);
            } catch (CallException e) {
                return;
            }
        }

        void fail(CallException error) {
            CallException reason;
            synchronized (this) {
                if (failure == null) {
                    failure = error;
                }
                reason = failure;
            }
            List<Pending> waiting = new ArrayList<>(pending.values());
            pending.clear();
            for (Pending entry : waiting) {
                entry.future.completeExceptionally(reason);
            }
        }

        void shutdown(Duration grace) {
            if (isOpen()) {
                try {
                    await(send("shutdown", new LinkedHashMap<>(), null), "shutdown", grace);
                } catch (CallException e) {
                    terminate(grace);
                    return;
                }
            }
            terminate(grace);
        }

        void terminate(Duration grace) {
            try {
                synchronized (writer) {
                    writer.close();
                }
            } catch (IOException e) {
                process.destroy();
            }
            try {
                if (!process.waitFor(grace.toMillis(), TimeUnit.MILLISECONDS)) {
                    process.destroy();
                    if (!process.waitFor(SHUTDOWN_GRACE.toMillis(), TimeUnit.MILLISECONDS)) {
                        process.destroyForcibly();
                    }
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                process.destroyForcibly();
            }
            fail(new CallException(CallException.Kind.DISCONNECTED, "connection to nurt serve is closed"));
        }

        private void read() {
            CallException reason;
            try (BufferedReader in = new BufferedReader(
                    new InputStreamReader(process.getInputStream(), StandardCharsets.UTF_8))) {
                reason = readMessages(in);
            } catch (IOException e) {
                reason = new CallException(CallException.Kind.DISCONNECTED,
                        "cannot read from nurt serve: " + e.getMessage(), e);
            }
            fail(reason);
        }

        private CallException readMessages(BufferedReader in) throws IOException {
            StringBuilder line = new StringBuilder();
            while (true) {
                int c = in.read();
                if (c == -1) {
                    if (!line.toString().isBlank()) {
                        CallException error = handle(line.toString());
                        if (error != null) {
                            return error;
                        }
                    }
                    return new CallException(CallException.Kind.DISCONNECTED, "nurt serve closed the connection" + exitStatus());
                }
                if (c != '\n') {
                    if (line.length() >= MAX_MESSAGE_CHARS) {
                        return new CallException(CallException.Kind.PROTOCOL,
                                "message from nurt serve exceeds " + MAX_MESSAGE_CHARS + " characters");
                    }
                    line.append((char) c);
                    continue;
                }
                String text = line.toString();
                line.setLength(0);
                if (text.isBlank()) {
                    continue;
                }
                CallException error = handle(text);
                if (error != null) {
                    return error;
                }
            }
        }

        private String exitStatus() {
            try {
                if (process.waitFor(1, TimeUnit.SECONDS)) {
                    return " (exit code " + process.exitValue() + ")";
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
            return "";
        }

        private CallException handle(String text) {
            Object message;
            try {
                message = Json.parse(text);
            } catch (JsonException e) {
                return new CallException(CallException.Kind.PROTOCOL,
                        "nurt serve sent a message that is not valid JSON: " + e.getMessage(), e);
            }
            dispatch(message);
            return null;
        }

        private void dispatch(Object message) {
            String method = Json.string(message, "method");
            if ("nurt/event".equals(method)) {
                Map<String, Object> params = Json.object(Json.member(message, "params"));
                if (params == null) {
                    return;
                }
                Long id = requestId(params.get("request_id"));
                Pending target = id == null ? null : pending.get(id);
                if (target != null && target.events != null) {
                    target.events.accept(params);
                } else if (events != null) {
                    events.accept(params);
                }
                return;
            }
            if (method != null) {
                return;
            }
            Long id = requestId(Json.member(message, "id"));
            Pending target = id == null ? null : pending.remove(id);
            if (target == null) {
                return;
            }
            Object error = Json.member(message, "error");
            if (error != null) {
                target.future.completeExceptionally(rpcError(error));
            } else {
                target.future.complete(Json.member(message, "result"));
            }
        }
    }
}
