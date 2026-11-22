// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt.internal;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.math.BigInteger;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

class JsonTest {
    @Test
    void readsEveryKindOfValue() {
        Object value = Json.parse(
                " {\"a\": [1, -2, 3.5, 1e3, true, false, null], \"b\": {\"c\": \"d\"}, \"e\": \"\"} ");
        Map<String, Object> object = Json.object(value);
        assertEquals(Arrays.asList(1L, -2L, 3.5, 1000.0, true, false, null), object.get("a"));
        assertEquals(Map.of("c", "d"), object.get("b"));
        assertEquals("", object.get("e"));
    }

    @Test
    void keepsTheOrderOfMembers() {
        Map<String, Object> object = Json.object(Json.parse("{\"z\": 1, \"a\": 2, \"m\": 3}"));
        assertEquals(List.of("z", "a", "m"), new ArrayList<>(object.keySet()));
    }

    @Test
    void repeatedMemberKeepsTheLastValue() {
        assertEquals(Map.of("a", 2L), Json.parse("{\"a\": 1, \"a\": 2}"));
    }

    @Test
    void decodesEscapesAndSurrogatePairs() {
        assertEquals("\"\\/\b\f\n\r\t", Json.parse("\"\\\"\\\\\\/\\b\\f\\n\\r\\t\""));
        assertEquals("\u00e9\ud83d\ude00", Json.parse("\"\\u00E9\\ud83d\\ude00\""));
        assertEquals("zażółć", Json.parse("\"zażółć\""));
    }

    @Test
    void integersBeyondLongBecomeBigIntegers() {
        assertEquals(Long.MAX_VALUE, Json.parse("9223372036854775807"));
        assertEquals(new BigInteger("9223372036854775808"), Json.parse("9223372036854775808"));
        assertEquals(Long.MIN_VALUE, Json.parse("-9223372036854775808"));
        assertEquals(-0.0, Json.parse("-0.0"));
        assertEquals(0L, Json.parse("-0"));
    }

    @ParameterizedTest
    @ValueSource(strings = {
        "", " ", "{", "[1,]", "{\"a\":1,}", "{a:1}", "01", "1.", ".5", "1e", "+1", "--1",
        "\"abc", "\"\\x\"", "\"\\u12\"", "\"\\u12G4\"", "tru", "nul", "[1 2]", "{\"a\" 1}",
        "1 2", "\"a\u0001\"", "'a'", "NaN", "Infinity", "[", "]"
    })
    void rejectsInvalidText(String text) {
        assertThrows(JsonException.class, () -> Json.parse(text));
    }

    @Test
    void rejectsNestingBeyondTheLimit() {
        String deep = "[".repeat(Json.MAX_DEPTH + 1) + "]".repeat(Json.MAX_DEPTH + 1);
        JsonException error = assertThrows(JsonException.class, () -> Json.parse(deep));
        assertTrue(error.getMessage().contains("nesting"));
        String allowed = "[".repeat(Json.MAX_DEPTH) + "]".repeat(Json.MAX_DEPTH);
        Json.parse(allowed);
    }

    @Test
    void writesCompactTextThatReadsBack() {
        Map<String, Object> value = new LinkedHashMap<>();
        value.put("text", "line\nquote\" back\\ tab\t \u0001 zażółć \ud83d\ude00");
        value.put("list", Arrays.asList(1L, 2.5, true, null, new BigInteger("123456789012345678901")));
        value.put("empty", Map.of());
        String text = Json.write(value);
        assertTrue(text.indexOf('\n') < 0, "one line");
        assertTrue(text.contains("\\u0001"));
        assertEquals(value, Json.parse(text));
    }

    @Test
    void writesNullAndNumbers() {
        assertEquals("null", Json.write(null));
        assertEquals("[1,2.5,-3]", Json.write(List.of(1, 2.5, -3L)));
        assertEquals("{}", Json.write(Map.of()));
    }

    @Test
    void refusesValuesWithoutJsonForm() {
        assertThrows(IllegalArgumentException.class, () -> Json.write(Double.NaN));
        assertThrows(IllegalArgumentException.class, () -> Json.write(Double.POSITIVE_INFINITY));
        assertThrows(IllegalArgumentException.class, () -> Json.write(new Object()));
        assertThrows(IllegalArgumentException.class, () -> Json.write(Map.of(1, 2)));
    }

    @Test
    void accessorsTolerateOtherShapes() {
        assertNull(Json.object(List.of()));
        assertNull(Json.member("text", "a"));
        assertNull(Json.string(Map.of("a", 1L), "a"));
        assertEquals("x", Json.string(Map.of("a", "x"), "a"));
    }
}
