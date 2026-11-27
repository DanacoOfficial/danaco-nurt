// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/main/java/pl/danacogroup/nurt/spi/BackendSettings.java
// Responsibility:   Source file bindings/java/nurt/src/main/java/pl/danacogroup/nurt/spi/BackendSettings.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt.spi;

import java.nio.file.Path;
import java.time.Duration;
import java.util.List;
import java.util.Map;
import java.util.function.Consumer;

/**
 * Settings of a backend.
 *
 * @param executable path of the {@code nurt} executable, or {@code null}
 * @param command full command line of a server, or an empty list
 * @param environment variables added to the environment of a server process
 * @param library path of {@code libnurt}, or {@code null}
 * @param config configuration {@code nurt-config/1} applied over the
 *     configuration files, or {@code null}
 * @param session default session identifier, or {@code null}
 * @param language default language tag, or {@code null}
 * @param requestTimeout time limit of each response, or {@code null}
 * @param events receiver of the events that belong to no execution
 */
public record BackendSettings(
        Path executable,
        List<String> command,
        Map<String, String> environment,
        Path library,
        Map<String, Object> config,
        String session,
        String language,
        Duration requestTimeout,
        Consumer<Map<String, Object>> events) {

    /** Copies the lists and maps of the settings. */
    public BackendSettings {
        command = List.copyOf(command);
        environment = Map.copyOf(environment);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
