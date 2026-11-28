// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/main/java/pl/danacogroup/nurt/NurtClientOptions.java
// Responsibility:   Source file bindings/java/nurt/src/main/java/pl/danacogroup/nurt/NurtClientOptions.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt;

import java.nio.file.Path;
import java.time.Duration;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;

/**
 * Settings of a {@link NurtClient}. The setters change these options and return them.
 *
 * <pre>{@code
 * NurtClientOptions options = new NurtClientOptions()
 *         .backend(BackendKind.PROCESS)
 *         .language(Request.Language.EN)
 *         .requestTimeout(Duration.ofMinutes(5));
 * }</pre>
 */
public final class NurtClientOptions {
    private BackendKind backend = BackendKind.AUTO;
    private Path executable;
    private List<String> command = List.of();
    private Map<String, String> environment = Map.of();
    private Path library;
    private Map<String, Object> config;
    private String session;
    private Request.Language language;
    private Duration requestTimeout;

    /** Options with the default values: backend {@link BackendKind#AUTO}, no timeout. */
    public NurtClientOptions() {
    }

    /**
     * Backend of the client.
     *
     * @param backend the backend
     * @return these options
     */
    public NurtClientOptions backend(BackendKind backend) {
        this.backend = Objects.requireNonNull(backend, "backend");
        return this;
    }

    /**
     * Path of the {@code nurt} executable. Without it the executable named by
     * {@code NURT_EXECUTABLE}, the executable bundled in the package and the
     * first {@code nurt} in {@code PATH} are tried in this order.
     *
     * @param executable the path
     * @return these options
     */
    public NurtClientOptions executable(Path executable) {
        this.executable = executable;
        return this;
    }

    /**
     * Full command line of a server that speaks {@code nurt-rpc/1} on its
     * standard streams, used instead of {@code <executable> serve --stdio}.
     *
     * @param command the program and its arguments
     * @return these options
     */
    public NurtClientOptions command(List<String> command) {
        this.command = List.copyOf(command);
        return this;
    }

    /**
     * Variables added to the environment of the server process, for example
     * {@code NURT_CONFIG} or {@code NURT_STATE_DIR}.
     *
     * @param environment names and values
     * @return these options
     */
    public NurtClientOptions environment(Map<String, String> environment) {
        this.environment = Map.copyOf(environment);
        return this;
    }

    /**
     * Path of {@code libnurt} for the library backend. Without it the library
     * named by {@code NURT_LIBRARY} and the library bundled in the package are
     * tried.
     *
     * @param library the path
     * @return these options
     */
    public NurtClientOptions library(Path library) {
        this.library = library;
        return this;
    }

    /**
     * Configuration {@code nurt-config/1} applied over the configuration files.
     *
     * @param config the configuration as parsed JSON
     * @return these options
     */
    public NurtClientOptions config(Map<String, Object> config) {
        this.config = config == null ? null : new LinkedHashMap<>(config);
        return this;
    }

    /**
     * Default session identifier of requests.
     *
     * @param session the identifier
     * @return these options
     */
    public NurtClientOptions session(String session) {
        this.session = session;
        return this;
    }

    /**
     * Default language of messages and help.
     *
     * @param language the language
     * @return these options
     */
    public NurtClientOptions language(Request.Language language) {
        this.language = language;
        return this;
    }

    /**
     * Time limit of each response; a request that passes it fails with
     * {@link NurtCallException.Kind#TIMEOUT} and is cancelled on the server.
     *
     * @param requestTimeout the limit, or {@code null} to wait without limit
     * @return these options
     */
    public NurtClientOptions requestTimeout(Duration requestTimeout) {
        this.requestTimeout = requestTimeout;
        return this;
    }

    BackendKind backend() {
        return backend;
    }

    Path executable() {
        return executable;
    }

    List<String> command() {
        return command;
    }

    Map<String, String> environment() {
        return environment;
    }

    Path library() {
        return library;
    }

    Map<String, Object> config() {
        return config;
    }

    String session() {
        return session;
    }

    Request.Language language() {
        return language;
    }

    Duration requestTimeout() {
        return requestTimeout;
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
