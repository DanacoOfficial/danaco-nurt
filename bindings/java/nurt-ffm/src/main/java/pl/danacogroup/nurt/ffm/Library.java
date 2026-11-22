// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.ffm;

import static java.lang.foreign.ValueLayout.ADDRESS;
import static java.lang.foreign.ValueLayout.JAVA_INT;
import static java.lang.foreign.ValueLayout.JAVA_LONG;

import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemoryLayout;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.StructLayout;
import java.lang.foreign.SymbolLookup;
import java.lang.invoke.MethodHandle;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import pl.danacogroup.nurt.NurtCallException;
import pl.danacogroup.nurt.CallStatus;
import pl.danacogroup.nurt.internal.Json;

/**
 * Functions of {@code libnurt} as declared in {@code nurt.h}.
 *
 * <p>Every function returning {@code nurt_status} is bound to a method
 * handle returning {@code int}; buffers are {@code nurt_buf} structures
 * ({@code ptr}, {@code len}, {@code cap}) allocated by the caller and
 * released with {@code nurt_buf_free}.
 */
final class Library {
    /** Interface version of the header this backend is written against. */
    static final int ABI_MAJOR = 1;
    static final int ABI_MINOR = 0;
    static final int ABI_VERSION = (ABI_MAJOR << 16) | ABI_MINOR;

    /** {@code NURT_WAIT_FOREVER} ({@code UINT32_MAX}) as a Java {@code int}. */
    static final int WAIT_FOREVER = -1;

    /** Layout of {@code nurt_buf}; {@code size_t} is 64-bit on every supported platform. */
    static final StructLayout BUF = MemoryLayout.structLayout(
            ADDRESS.withName("ptr"), JAVA_LONG.withName("len"), JAVA_LONG.withName("cap"));

    /** Descriptor of {@code nurt_event_cb}. */
    static final FunctionDescriptor EVENT_CALLBACK = FunctionDescriptor.ofVoid(ADDRESS, ADDRESS);

    private static final FunctionDescriptor STATUS_E_STR_BUF = FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, ADDRESS);
    private static final FunctionDescriptor STATUS_E_STR_INT_BUF =
            FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, JAVA_INT, ADDRESS);

    final Path path;
    final MethodHandle abiVersion;
    final MethodHandle abiCheck;
    final MethodHandle info;
    final MethodHandle engineOpen;
    final MethodHandle engineClose;
    final MethodHandle parse;
    final MethodHandle print;
    final MethodHandle correct;
    final MethodHandle check;
    final MethodHandle runJson;
    final MethodHandle execStart;
    final MethodHandle execWait;
    final MethodHandle execCancel;
    final MethodHandle execFree;
    final MethodHandle page;
    final MethodHandle catalog;
    final MethodHandle describe;
    final MethodHandle help;
    final MethodHandle schema;
    final MethodHandle history;
    final MethodHandle space;
    final MethodHandle bufFree;
    final MethodHandle lastError;

    private Library(Path path, SymbolLookup lookup) {
        this.path = path;
        Linker linker = Linker.nativeLinker();
        abiVersion = bind(linker, lookup, "nurt_abi_version", FunctionDescriptor.of(JAVA_INT));
        abiCheck = bind(linker, lookup, "nurt_abi_check", FunctionDescriptor.of(JAVA_INT, JAVA_INT));
        info = bind(linker, lookup, "nurt_info", FunctionDescriptor.of(JAVA_INT, ADDRESS));
        engineOpen = bind(linker, lookup, "nurt_engine_open", FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS));
        engineClose = bind(linker, lookup, "nurt_engine_close", FunctionDescriptor.ofVoid(ADDRESS));
        parse = bind(linker, lookup, "nurt_parse",
                FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, JAVA_INT, ADDRESS, ADDRESS));
        print = bind(linker, lookup, "nurt_print", STATUS_E_STR_INT_BUF);
        correct = bind(linker, lookup, "nurt_correct", STATUS_E_STR_INT_BUF);
        check = bind(linker, lookup, "nurt_check", FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, ADDRESS, ADDRESS));
        runJson = bind(linker, lookup, "nurt_run_json", STATUS_E_STR_BUF);
        execStart = bind(linker, lookup, "nurt_exec_start",
                FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, ADDRESS, ADDRESS, ADDRESS));
        execWait = bind(linker, lookup, "nurt_exec_wait", FunctionDescriptor.of(JAVA_INT, ADDRESS, JAVA_INT, ADDRESS));
        execCancel = bind(linker, lookup, "nurt_exec_cancel", FunctionDescriptor.of(JAVA_INT, ADDRESS));
        execFree = bind(linker, lookup, "nurt_exec_free", FunctionDescriptor.ofVoid(ADDRESS));
        page = bind(linker, lookup, "nurt_page", STATUS_E_STR_INT_BUF);
        catalog = bind(linker, lookup, "nurt_catalog", STATUS_E_STR_BUF);
        describe = bind(linker, lookup, "nurt_describe", STATUS_E_STR_BUF);
        help = bind(linker, lookup, "nurt_help", STATUS_E_STR_BUF);
        schema = bind(linker, lookup, "nurt_schema", FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS));
        history = bind(linker, lookup, "nurt_history", STATUS_E_STR_BUF);
        space = bind(linker, lookup, "nurt_space", STATUS_E_STR_BUF);
        bufFree = bind(linker, lookup, "nurt_buf_free", FunctionDescriptor.ofVoid(ADDRESS));
        lastError = bind(linker, lookup, "nurt_last_error", FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS));
    }

    /**
     * Loads the library for the lifetime of the process.
     *
     * @param path the library
     * @return the bound functions
     * @throws NurtCallException of kind {@link NurtCallException.Kind#IO} when the
     *     library cannot be loaded or lacks a function of the header
     */
    static Library load(Path path) {
        try {
            return new Library(path, SymbolLookup.libraryLookup(path, Arena.global()));
        } catch (IllegalArgumentException e) {
            throw new NurtCallException(NurtCallException.Kind.IO,
                    "cannot load " + path + ": " + e.getMessage() + "; check NURT_LIBRARY", e);
        }
    }

    private static MethodHandle bind(Linker linker, SymbolLookup lookup, String name, FunctionDescriptor descriptor) {
        MemorySegment symbol = lookup.find(name).orElseThrow(() -> new IllegalArgumentException(
                "the library has no function " + name));
        return linker.downcallHandle(symbol, descriptor);
    }

    /**
     * Checks that the library implements the interface of {@code nurt.h} 1.0.
     *
     * @throws NurtCallException of kind {@link NurtCallException.Kind#INCOMPATIBLE}
     *     when the major versions differ or the minor version is lower
     */
    void checkAbi() {
        int status = invokeInt(abiCheck, ABI_VERSION);
        if (status != 0) {
            int found = invokeInt(abiVersion);
            throw new NurtCallException(NurtCallException.Kind.INCOMPATIBLE,
                    path + " implements interface " + (found >>> 16) + "." + (found & 0xffff)
                            + ", this backend needs " + ABI_MAJOR + "." + ABI_MINOR + " or a later minor version");
        }
    }

    /**
     * Throws the failure of a call that returned a status other than {@code NURT_OK}.
     *
     * @param status the returned status
     * @param engine the engine, or {@link MemorySegment#NULL}
     * @param function name of the C function
     */
    void verify(int status, MemorySegment engine, String function) {
        if (status == 0) {
            return;
        }
        String detail = function + " failed";
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment buffer = arena.allocate(BUF);
            if (invokeInt(lastError, engine, buffer) == 0) {
                String json = take(buffer);
                String message = json.isEmpty() ? null : Json.string(Json.parse(json), "message");
                if (message != null) {
                    detail = function + ": " + message;
                }
            }
        }
        NurtCallException.Kind kind = CallStatus.fromCode(status)
                .map(NurtCallException.Kind::of)
                .orElse(NurtCallException.Kind.INTERNAL);
        throw new NurtCallException(kind, detail + " (status " + status + ")");
    }

    /**
     * Reads a buffer as UTF-8 text and releases it.
     *
     * @param buffer a {@code nurt_buf} filled by the library
     * @return the text; empty for an empty buffer
     */
    String take(MemorySegment buffer) {
        try {
            MemorySegment pointer = buffer.get(ADDRESS, BUF.byteOffset(MemoryLayout.PathElement.groupElement("ptr")));
            long length = buffer.get(JAVA_LONG, BUF.byteOffset(MemoryLayout.PathElement.groupElement("len")));
            if (pointer.equals(MemorySegment.NULL) || length == 0) {
                return "";
            }
            return new String(pointer.reinterpret(length).toArray(java.lang.foreign.ValueLayout.JAVA_BYTE),
                    StandardCharsets.UTF_8);
        } finally {
            invokeVoid(bufFree, buffer);
        }
    }

    static int invokeInt(MethodHandle handle, Object... args) {
        try {
            return (int) handle.invokeWithArguments(args);
        } catch (RuntimeException | Error e) {
            throw e;
        } catch (Throwable e) {
            throw new NurtCallException(NurtCallException.Kind.INTERNAL, "call of libnurt failed: " + e, e);
        }
    }

    static void invokeVoid(MethodHandle handle, Object... args) {
        try {
            handle.invokeWithArguments(args);
        } catch (RuntimeException | Error e) {
            throw e;
        } catch (Throwable e) {
            throw new NurtCallException(NurtCallException.Kind.INTERNAL, "call of libnurt failed: " + e, e);
        }
    }
}
