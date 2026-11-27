// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/main/java/pl/danacogroup/nurt/internal/Json.java
// Responsibility:   Source file bindings/java/nurt/src/main/java/pl/danacogroup/nurt/internal/Json.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt.internal;

import java.math.BigInteger;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Reader and writer of JSON text (RFC 8259).
 *
 * <p>Values map to Java as follows: object to {@link LinkedHashMap} with the
 * order of the text, array to {@link ArrayList}, string to {@link String},
 * integer to {@link Long} or, outside its range, {@link BigInteger}, number
 * with a fraction or an exponent to {@link Double}, {@code true} and
 * {@code false} to {@link Boolean}, {@code null} to {@code null}. A repeated
 * member name keeps the last value.
 */
public final class Json {
    /** Deepest nesting of arrays and objects accepted by the reader. */
    public static final int MAX_DEPTH = 512;

    private Json() {
    }

    /**
     * Reads one JSON value; only whitespace may surround it.
     *
     * @param text JSON text
     * @return the value
     * @throws JsonException when the text is not valid JSON
     */
    public static Object parse(String text) {
        Reader reader = new Reader(text);
        reader.skipWhitespace();
        Object value = reader.value(0);
        reader.skipWhitespace();
        if (!reader.atEnd()) {
            throw reader.error("unexpected text after the value");
        }
        return value;
    }

    /**
     * Writes a value as compact JSON text without line breaks.
     *
     * @param value a map with string keys, a list, a string, a number, a
     *     boolean or {@code null}
     * @return JSON text
     * @throws IllegalArgumentException when the value has another type or is
     *     a non-finite number
     */
    public static String write(Object value) {
        StringBuilder out = new StringBuilder();
        writeValue(out, value);
        return out.toString();
    }

    /**
     * The value as a JSON object.
     *
     * @param value a parsed value
     * @return the object, or {@code null} when the value is not an object
     */
    @SuppressWarnings("unchecked")
    public static Map<String, Object> object(Object value) {
        return value instanceof Map<?, ?> map ? (Map<String, Object>) map : null;
    }

    /**
     * Member of an object.
     *
     * @param value a parsed value
     * @param name member name
     * @return the member, or {@code null} when the value is not an object or
     *     has no such member
     */
    public static Object member(Object value, String name) {
        Map<String, Object> object = object(value);
        return object == null ? null : object.get(name);
    }

    /**
     * String member of an object.
     *
     * @param value a parsed value
     * @param name member name
     * @return the string, or {@code null} when absent or not a string
     */
    public static String string(Object value, String name) {
        return member(value, name) instanceof String text ? text : null;
    }

    private static void writeValue(StringBuilder out, Object value) {
        if (value == null) {
            out.append("null");
        } else if (value instanceof String text) {
            writeString(out, text);
        } else if (value instanceof Boolean flag) {
            out.append(flag.booleanValue());
        } else if (value instanceof Double || value instanceof Float) {
            double number = ((Number) value).doubleValue();
            if (Double.isNaN(number) || Double.isInfinite(number)) {
                throw new IllegalArgumentException("JSON has no representation of " + number);
            }
            out.append(number);
        } else if (value instanceof Number number) {
            out.append(number);
        } else if (value instanceof Map<?, ?> map) {
            out.append('{');
            boolean first = true;
            for (Map.Entry<?, ?> entry : map.entrySet()) {
                if (!(entry.getKey() instanceof String name)) {
                    throw new IllegalArgumentException("JSON object keys must be strings");
                }
                if (!first) {
                    out.append(',');
                }
                first = false;
                writeString(out, name);
                out.append(':');
                writeValue(out, entry.getValue());
            }
            out.append('}');
        } else if (value instanceof List<?> list) {
            out.append('[');
            for (int i = 0; i < list.size(); i++) {
                if (i > 0) {
                    out.append(',');
                }
                writeValue(out, list.get(i));
            }
            out.append(']');
        } else {
            throw new IllegalArgumentException(
                    "no JSON form for " + value.getClass().getName());
        }
    }

    private static void writeString(StringBuilder out, String text) {
        out.append('"');
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            switch (c) {
                case '"' -> out.append("\\\"");
                case '\\' -> out.append("\\\\");
                case '\n' -> out.append("\\n");
                case '\r' -> out.append("\\r");
                case '\t' -> out.append("\\t");
                case '\b' -> out.append("\\b");
                case '\f' -> out.append("\\f");
                default -> {
                    if (c < 0x20) {
                        out.append(String.format("\\u%04x", (int) c));
                    } else {
                        out.append(c);
                    }
                }
            }
        }
        out.append('"');
    }

    private static final class Reader {
        private final String text;
        private int position;

        Reader(String text) {
            this.text = text;
        }

        boolean atEnd() {
            return position >= text.length();
        }

        JsonException error(String message) {
            return new JsonException(message + " at offset " + position);
        }

        void skipWhitespace() {
            while (!atEnd()) {
                char c = text.charAt(position);
                if (c != ' ' && c != '\t' && c != '\n' && c != '\r') {
                    return;
                }
                position++;
            }
        }

        Object value(int depth) {
            if (atEnd()) {
                throw error("unexpected end of text");
            }
            char c = text.charAt(position);
            return switch (c) {
                case '{' -> object(depth + 1);
                case '[' -> array(depth + 1);
                case '"' -> string();
                case 't' -> literal("true", Boolean.TRUE);
                case 'f' -> literal("false", Boolean.FALSE);
                case 'n' -> literal("null", null);
                default -> {
                    if (c == '-' || (c >= '0' && c <= '9')) {
                        yield number();
                    }
                    throw error("unexpected character '" + c + "'");
                }
            };
        }

        private void enter(int depth) {
            if (depth > MAX_DEPTH) {
                throw error("nesting deeper than " + MAX_DEPTH);
            }
            position++;
        }

        private Map<String, Object> object(int depth) {
            enter(depth);
            Map<String, Object> object = new LinkedHashMap<>();
            skipWhitespace();
            if (consume('}')) {
                return object;
            }
            while (true) {
                skipWhitespace();
                if (atEnd() || text.charAt(position) != '"') {
                    throw error("expected a member name");
                }
                String name = string();
                skipWhitespace();
                expect(':');
                skipWhitespace();
                object.put(name, value(depth));
                skipWhitespace();
                if (consume('}')) {
                    return object;
                }
                expect(',');
            }
        }

        private List<Object> array(int depth) {
            enter(depth);
            List<Object> array = new ArrayList<>();
            skipWhitespace();
            if (consume(']')) {
                return array;
            }
            while (true) {
                skipWhitespace();
                array.add(value(depth));
                skipWhitespace();
                if (consume(']')) {
                    return array;
                }
                expect(',');
            }
        }

        private boolean consume(char c) {
            if (!atEnd() && text.charAt(position) == c) {
                position++;
                return true;
            }
            return false;
        }

        private void expect(char c) {
            if (!consume(c)) {
                throw error("expected '" + c + "'");
            }
        }

        private Object literal(String word, Object value) {
            if (!text.startsWith(word, position)) {
                throw error("invalid literal");
            }
            position += word.length();
            return value;
        }

        private String string() {
            position++;
            StringBuilder out = new StringBuilder();
            while (true) {
                if (atEnd()) {
                    throw error("unterminated string");
                }
                char c = text.charAt(position++);
                if (c == '"') {
                    return out.toString();
                }
                if (c < 0x20) {
                    throw error("control character in a string");
                }
                if (c != '\\') {
                    out.append(c);
                    continue;
                }
                if (atEnd()) {
                    throw error("unterminated escape");
                }
                char escaped = text.charAt(position++);
                switch (escaped) {
                    case '"' -> out.append('"');
                    case '\\' -> out.append('\\');
                    case '/' -> out.append('/');
                    case 'b' -> out.append('\b');
                    case 'f' -> out.append('\f');
                    case 'n' -> out.append('\n');
                    case 'r' -> out.append('\r');
                    case 't' -> out.append('\t');
                    case 'u' -> out.append(unicodeEscape());
                    default -> throw error("invalid escape '\\" + escaped + "'");
                }
            }
        }

        private char unicodeEscape() {
            if (position + 4 > text.length()) {
                throw error("incomplete unicode escape");
            }
            int code = 0;
            for (int i = 0; i < 4; i++) {
                int digit = Character.digit(text.charAt(position + i), 16);
                if (digit < 0) {
                    throw error("invalid unicode escape");
                }
                code = code * 16 + digit;
            }
            position += 4;
            return (char) code;
        }

        private Number number() {
            int start = position;
            consume('-');
            if (consume('0')) {
                if (!atEnd() && Character.isDigit(text.charAt(position))) {
                    throw error("leading zero in a number");
                }
            } else {
                digits();
            }
            boolean integer = true;
            if (consume('.')) {
                integer = false;
                digits();
            }
            if (!atEnd() && (text.charAt(position) == 'e' || text.charAt(position) == 'E')) {
                integer = false;
                position++;
                if (!consume('+')) {
                    consume('-');
                }
                digits();
            }
            String literal = text.substring(start, position);
            if (!integer) {
                return Double.valueOf(literal);
            }
            BigInteger value = new BigInteger(literal);
            return value.bitLength() < Long.SIZE ? (Number) value.longValue() : value;
        }

        private void digits() {
            int start = position;
            while (!atEnd() && text.charAt(position) >= '0' && text.charAt(position) <= '9') {
                position++;
            }
            if (position == start) {
                throw error("expected a digit");
            }
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
