// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.ffm;

import static java.lang.foreign.ValueLayout.ADDRESS;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.lang.foreign.Arena;
import java.lang.foreign.MemorySegment;
import java.lang.invoke.MethodHandle;
import java.nio.file.Path;
import java.util.Map;
import java.util.Set;
import org.junit.jupiter.api.Assumptions;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import pl.danacogroup.nurt.NurtCallException;
import pl.danacogroup.nurt.internal.Json;

/**
 * Status codes returned by each function of {@code libnurt} against the
 * table "Possible status codes of each function" in {@code nurt.h}.
 *
 * <p>The tests call the library named by {@code NURT_LIBRARY} and are
 * skipped when the variable is not set. Every call uses invalid arguments or
 * needs no engine, so no state directory is opened.
 */
class StatusMatrixTest {
    private static final int OK = 0;
    private static final int TIMEOUT = 1;
    private static final int CANCELLED = 2;
    private static final int ARG = 3;
    private static final int ABI = 4;
    private static final int IO = 5;
    private static final int INTERNAL = 6;

    /** The table of {@code nurt.h}: function to the statuses it may return. */
    private static final Map<String, Set<Integer>> PERMITTED = Map.ofEntries(
            Map.entry("nurt_abi_check", Set.of(OK, ABI)),
            Map.entry("nurt_info", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_engine_open", Set.of(OK, ARG, IO, INTERNAL)),
            Map.entry("nurt_parse", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_print", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_correct", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_check", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_run_json", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_exec_start", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_exec_wait", Set.of(OK, TIMEOUT, CANCELLED, ARG, INTERNAL)),
            Map.entry("nurt_exec_cancel", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_page", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_catalog", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_describe", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_help", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_schema", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_history", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_space", Set.of(OK, ARG, INTERNAL)),
            Map.entry("nurt_last_error", Set.of(OK, ARG)));

    private static final MemorySegment NULL = MemorySegment.NULL;

    private static Library library;

    @BeforeAll
    static void loadLibrary() {
        String path = System.getenv("NURT_LIBRARY");
        Assumptions.assumeTrue(path != null && !path.isEmpty(), "NURT_LIBRARY is not set");
        library = Library.load(Path.of(path).toAbsolutePath());
    }

    private static int call(String function, MethodHandle handle, Object... arguments) {
        int status = Library.invokeInt(handle, arguments);
        assertTrue(PERMITTED.get(function).contains(status),
                function + " returned " + status + ", which nurt.h does not list for it");
        return status;
    }

    private static void rejectsWithArg(String function, MethodHandle handle, Object... arguments) {
        assertEquals(ARG, call(function, handle, arguments), function);
    }

    @Test
    void abiCheckAcceptsTheHeaderVersionAndRejectsAnotherMajor() {
        assertEquals(OK, call("nurt_abi_check", library.abiCheck, Library.ABI_VERSION));
        assertEquals(ABI, call("nurt_abi_check", library.abiCheck, (Library.ABI_MAJOR + 1) << 16));
        int version = Library.invokeInt(library.abiVersion);
        assertEquals(Library.ABI_MAJOR, version >>> 16);
    }

    @Test
    void infoReturnsJsonAndRejectsNullOutput() {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment buffer = arena.allocate(Library.BUF);
            assertEquals(OK, call("nurt_info", library.info, buffer));
            assertTrue(Json.object(Json.parse(library.take(buffer))) != null, "nurt_info returns an object");
            rejectsWithArg("nurt_info", library.info, NULL);
        }
    }

    @Test
    void engineOpenRejectsNullOutputAndInvalidConfiguration() {
        try (Arena arena = Arena.ofConfined()) {
            rejectsWithArg("nurt_engine_open", library.engineOpen, NULL, NULL);
            MemorySegment out = arena.allocate(ADDRESS);
            rejectsWithArg("nurt_engine_open", library.engineOpen, arena.allocateFrom("{not json"), out);
            rejectsWithArg("nurt_engine_open", library.engineOpen, arena.allocateFrom("[1, 2]"), out);
            assertEquals(NULL, out.get(ADDRESS, 0), "no engine is returned after a failure");
        }
    }

    @Test
    void everyEngineFunctionRejectsANullEngine() {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment program = arena.allocateFrom("list \"/home/user\"");
            MemorySegment request = arena.allocateFrom("{\"nurt\":\"1\",\"program\":\"list\"}");
            MemorySegment first = arena.allocate(Library.BUF);
            MemorySegment second = arena.allocate(Library.BUF);
            MemorySegment execution = arena.allocate(ADDRESS);
            rejectsWithArg("nurt_parse", library.parse, NULL, program, 1, first, second);
            rejectsWithArg("nurt_print", library.print, NULL, arena.allocateFrom("{}"), 1, first);
            rejectsWithArg("nurt_correct", library.correct, NULL, program, 1, first);
            rejectsWithArg("nurt_check", library.check, NULL, request, first, second);
            rejectsWithArg("nurt_run_json", library.runJson, NULL, request, first);
            rejectsWithArg("nurt_exec_start", library.execStart, NULL, request, NULL, NULL, execution);
            rejectsWithArg("nurt_page", library.page, NULL, arena.allocateFrom("cursor"), 0, first);
            rejectsWithArg("nurt_catalog", library.catalog, NULL, NULL, first);
            rejectsWithArg("nurt_describe", library.describe, NULL, NULL, first);
            rejectsWithArg("nurt_help", library.help, NULL, NULL, first);
            rejectsWithArg("nurt_history", library.history, NULL, NULL, first);
            rejectsWithArg("nurt_space", library.space, NULL, NULL, first);
            Library.invokeVoid(library.bufFree, first);
            Library.invokeVoid(library.bufFree, second);
        }
    }

    @Test
    void executionFunctionsRejectANullHandle() {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment buffer = arena.allocate(Library.BUF);
            rejectsWithArg("nurt_exec_wait", library.execWait, NULL, 0, buffer);
            rejectsWithArg("nurt_exec_cancel", library.execCancel, NULL);
        }
    }

    @Test
    void schemaNeedsNoEngineAndRejectsAnUnknownName() {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment buffer = arena.allocate(Library.BUF);
            assertEquals(OK, call("nurt_schema", library.schema, arena.allocateFrom("request"), buffer));
            assertTrue(Json.object(Json.parse(library.take(buffer))) != null, "a schema is an object");
            rejectsWithArg("nurt_schema", library.schema, arena.allocateFrom("no-such-schema"), buffer);
            rejectsWithArg("nurt_schema", library.schema, NULL, buffer);
        }
    }

    @Test
    void lastErrorDescribesTheLastFailureOfTheThread() {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment buffer = arena.allocate(Library.BUF);
            rejectsWithArg("nurt_schema", library.schema, arena.allocateFrom("no-such-schema"), buffer);
            assertEquals(OK, call("nurt_last_error", library.lastError, NULL, buffer));
            Map<String, Object> error = Json.object(Json.parse(library.take(buffer)));
            assertEquals(Long.valueOf(ARG), error.get("status"));
            assertEquals("NURT_E_ARG", error.get("code"));
            rejectsWithArg("nurt_last_error", library.lastError, NULL, NULL);
        }
    }

    @Test
    void releaseFunctionsAcceptNull() {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment buffer = arena.allocate(Library.BUF);
            Library.invokeVoid(library.bufFree, buffer);
            Library.invokeVoid(library.bufFree, buffer);
            Library.invokeVoid(library.bufFree, NULL);
            Library.invokeVoid(library.execFree, NULL);
            Library.invokeVoid(library.engineClose, NULL);
        }
    }

    @Test
    void failedCallBecomesACallExceptionWithItsKind() {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment buffer = arena.allocate(Library.BUF);
            int status = call("nurt_schema", library.schema, arena.allocateFrom("no-such-schema"), buffer);
            assertNotEquals(OK, status);
            NurtCallException error = assertThrows(NurtCallException.class,
                    () -> library.verify(status, NULL, "nurt_schema"));
            assertEquals(NurtCallException.Kind.INVALID_ARGUMENT, error.kind());
            assertTrue(error.getMessage().startsWith("nurt_schema"), error.getMessage());
        }
    }
}
