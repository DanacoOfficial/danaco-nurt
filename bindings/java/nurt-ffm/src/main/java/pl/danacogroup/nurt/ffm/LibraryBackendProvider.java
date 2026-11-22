// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.ffm;

import java.nio.file.Path;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import pl.danacogroup.nurt.BackendKind;
import pl.danacogroup.nurt.NurtCallException;
import pl.danacogroup.nurt.internal.Artifacts;
import pl.danacogroup.nurt.spi.Backend;
import pl.danacogroup.nurt.spi.BackendProvider;
import pl.danacogroup.nurt.spi.BackendSettings;

/** Provider of the library backend, registered as a service. */
public final class LibraryBackendProvider implements BackendProvider {
    private static final Map<Path, Library> LOADED = new ConcurrentHashMap<>();

    /** Creates the provider; called by {@link java.util.ServiceLoader}. */
    public LibraryBackendProvider() {
    }

    @Override
    public BackendKind kind() {
        return BackendKind.LIBRARY;
    }

    @Override
    public boolean isAvailable(BackendSettings settings) {
        try {
            library(settings).checkAbi();
            return true;
        } catch (NurtCallException e) {
            return false;
        }
    }

    @Override
    public Backend open(BackendSettings settings) {
        Library library = library(settings);
        library.checkAbi();
        return LibraryBackend.open(library, settings);
    }

    private static Library library(BackendSettings settings) {
        Path path = Artifacts.locateLibrary(settings.library()).toAbsolutePath();
        return LOADED.computeIfAbsent(path, Library::load);
    }
}
