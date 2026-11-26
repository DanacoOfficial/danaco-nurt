// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.ffm;

import static java.lang.foreign.ValueLayout.JAVA_BYTE;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.lang.foreign.Arena;
import java.lang.foreign.MemoryLayout;
import java.lang.foreign.MemorySegment;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.Map;
import org.junit.jupiter.api.Test;
import pl.danacogroup.nurt.NurtCallException;

/**
 * Conversion of arguments for the library and the layout of its types; none of
 * these tests loads the library.
 */
class LibraryBackendArgumentsTest {
    private static Map<String, Object> params(String name, Object value) {
        Map<String, Object> params = new HashMap<>();
        params.put(name, value);
        return params;
    }

    private static void assertInvalid(Runnable action) {
        NurtCallException error = assertThrows(NurtCallException.class, action::run);
        assertEquals(NurtCallException.Kind.INVALID_ARGUMENT, error.kind());
    }

    @Test
    void textIsPassedAsUtf8FollowedByAZeroByte() {
        String text = "zażółć 😀";
        byte[] expected = text.getBytes(StandardCharsets.UTF_8);
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment copy = LibraryBackend.utf8(arena, text);
            assertEquals(expected.length + 1, copy.byteSize());
            assertEquals(text, copy.getString(0));
            assertEquals(0, copy.get(JAVA_BYTE, expected.length));
            assertEquals(1, LibraryBackend.utf8(arena, "").byteSize());
        }
    }

    @Test
    void textThatTheLibraryCouldNotReadUnchangedIsRefused() {
        try (Arena arena = Arena.ofConfined()) {
            assertInvalid(() -> LibraryBackend.utf8(arena, "find\u0000/etc"));
            assertInvalid(() -> LibraryBackend.utf8(arena, "a\uD800b"));
            assertInvalid(() -> LibraryBackend.utf8(arena, "a\uDC00"));
            assertInvalid(() -> LibraryBackend.text(arena, params("program", "x\u0000"), "program"));
        }
    }

    @Test
    void requiredTextMustBePresentAndAString() {
        try (Arena arena = Arena.ofConfined()) {
            assertInvalid(() -> LibraryBackend.text(arena, Map.of(), "program"));
            assertInvalid(() -> LibraryBackend.text(arena, params("program", 5L), "program"));
            assertEquals("echo", LibraryBackend.text(arena, params("program", "echo"), "program").getString(0));
            assertEquals(MemorySegment.NULL, LibraryBackend.optionalText(arena, Map.of(), "topic"));
            assertEquals("lists", LibraryBackend.optionalText(arena, params("topic", "lists"), "topic").getString(0));
        }
    }

    @Test
    void variantsAreNumberedFromOneAsInTheHeader() {
        assertEquals(1, LibraryBackend.variant(Map.of()));
        assertEquals(1, LibraryBackend.variant(params("variant", "a")));
        assertEquals(2, LibraryBackend.variant(params("variant", "b")));
        assertEquals(3, LibraryBackend.variant(params("variant", "c")));
        assertInvalid(() -> LibraryBackend.variant(params("variant", "d")));
        assertInvalid(() -> LibraryBackend.variant(params("variant", 1L)));
    }

    @Test
    void pageLimitIsAPositiveInteger() {
        assertEquals(0, LibraryBackend.limit(Map.of()));
        assertEquals(5, LibraryBackend.limit(params("limit", 5L)));
        assertEquals(7, LibraryBackend.limit(params("limit", 7)));
        assertEquals(Integer.MAX_VALUE, LibraryBackend.limit(params("limit", (long) Integer.MAX_VALUE)));
        assertInvalid(() -> LibraryBackend.limit(params("limit", 0L)));
        assertInvalid(() -> LibraryBackend.limit(params("limit", -1L)));
        assertInvalid(() -> LibraryBackend.limit(params("limit", Integer.MAX_VALUE + 1L)));
        assertInvalid(() -> LibraryBackend.limit(params("limit", "10")));
    }

    @Test
    void bufferHasTheLayoutOfNurtBuf() {
        assertEquals(24, Library.BUF.byteSize());
        assertEquals(0, Library.BUF.byteOffset(MemoryLayout.PathElement.groupElement("ptr")));
        assertEquals(8, Library.BUF.byteOffset(MemoryLayout.PathElement.groupElement("len")));
        assertEquals(16, Library.BUF.byteOffset(MemoryLayout.PathElement.groupElement("cap")));
    }

    @Test
    void versionAndWaitConstantsMatchTheHeader() {
        assertEquals(1 << 16, Library.ABI_VERSION);
        assertEquals(0xFFFFFFFFL, Library.WAIT_FOREVER & 0xFFFFFFFFL);
    }
}
