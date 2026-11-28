// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/main/java/pl/danacogroup/nurt/spi/BackendProvider.java
// Responsibility:   Source file bindings/java/nurt/src/main/java/pl/danacogroup/nurt/spi/BackendProvider.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt.spi;

import pl.danacogroup.nurt.BackendKind;

/**
 * Provider of an additional backend, found with {@link java.util.ServiceLoader}.
 *
 * <p>The artifact {@code pl.danaco-group:nurt-ffm} provides the library
 * backend ({@link BackendKind#LIBRARY}).
 */
public interface BackendProvider {
    /**
     * Kind of the backend.
     *
     * @return the kind
     */
    BackendKind kind();

    /**
     * Whether the backend can be opened with the settings, for example
     * whether the library is found and its interface version is compatible.
     *
     * @param settings the settings
     * @return {@code true} when {@link #open} is expected to succeed
     */
    boolean isAvailable(BackendSettings settings);

    /**
     * Opens the backend.
     *
     * @param settings the settings
     * @return the backend
     * @throws pl.danacogroup.nurt.NurtCallException when the backend cannot be opened
     */
    Backend open(BackendSettings settings);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
