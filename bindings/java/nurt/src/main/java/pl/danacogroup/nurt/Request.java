// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/java/nurt/src/main/java/pl/danacogroup/nurt/Request.java
// Responsibility:   Source file bindings/java/nurt/src/main/java/pl/danacogroup/nurt/Request.java.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package pl.danacogroup.nurt;

import java.time.Duration;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import pl.danacogroup.nurt.internal.Json;

/**
 * Request envelope ({@code nurt-request/1}): one input and the call options.
 *
 * <p>The input is program text, a syntax tree {@code nurt-ast/1}, the cursor
 * of a further page or the identifier of a correction candidate. Options that
 * are not set take their values from the configuration of the server. The
 * option methods change this request and return it.
 *
 * <pre>{@code
 * Request request = Request.program("find \"/home/user/src\" glob: \"*.go\"")
 *         .limit(20)
 *         .format(Request.Format.JSON);
 * }</pre>
 */
public final class Request {
    /** Version of the envelope written by this library. */
    public static final String ENVELOPE_VERSION = "1.0";

    /** Notation variant of program text. */
    public enum Variant {
        /** Word notation. */
        A,
        /** Call notation. */
        B,
        /** Named notation. */
        C;

        /**
         * Name in JSON documents.
         *
         * @return {@code a}, {@code b} or {@code c}
         */
        public String wire() {
            return name().toLowerCase(java.util.Locale.ROOT);
        }
    }

    /** Format of the rendered result. */
    public enum Format {
        /** Plain text. */
        TEXT,
        /** JSON only; no rendered text. */
        JSON,
        /** Markdown. */
        MARKDOWN,
        /** Compact text with the fewest characters. */
        COMPACT;

        /**
         * Name in JSON documents.
         *
         * @return the name in lower case
         */
        public String wire() {
            return name().toLowerCase(java.util.Locale.ROOT);
        }
    }

    /** Level of detail of the report. */
    public enum Verbosity {
        /** Status and error only. */
        QUIET,
        /** The default level. */
        NORMAL,
        /** Every field of the report. */
        DETAILED;

        /**
         * Name in JSON documents.
         *
         * @return the name in lower case
         */
        public String wire() {
            return name().toLowerCase(java.util.Locale.ROOT);
        }
    }

    /** Form of differences in the report. */
    public enum DiffMode {
        /** No differences. */
        NONE,
        /** Counts of changed lines per file. */
        STAT,
        /** Unified differences. */
        UNIFIED;

        /**
         * Name in JSON documents.
         *
         * @return the name in lower case
         */
        public String wire() {
            return name().toLowerCase(java.util.Locale.ROOT);
        }
    }

    /** Language of messages and help. */
    public enum Language {
        /** English. */
        EN,
        /** Polish. */
        PL;

        /**
         * Language tag in JSON documents.
         *
         * @return {@code en} or {@code pl}
         */
        public String wire() {
            return name().toLowerCase(java.util.Locale.ROOT);
        }
    }

    private final String inputName;
    private final Object input;
    private Variant variant;
    private final Map<String, Object> options = new LinkedHashMap<>();

    private Request(String inputName, Object input) {
        this.inputName = inputName;
        this.input = Objects.requireNonNull(input, inputName);
    }

    /**
     * Request that runs program text in any notation variant.
     *
     * @param text the program
     * @return the request
     */
    public static Request program(String text) {
        return new Request("program", text);
    }

    /**
     * Request that runs a syntax tree {@code nurt-ast/1}.
     *
     * @param tree the tree as parsed JSON
     * @return the request
     */
    public static Request ast(Map<String, Object> tree) {
        return new Request("ast", tree);
    }

    /**
     * Request that reads a further page of an earlier result.
     *
     * @param cursor the cursor
     * @return the request
     */
    public static Request cursor(String cursor) {
        return new Request("cursor", cursor);
    }

    /**
     * Request that accepts a candidate of an earlier {@code E_AMBIGUOUS} error.
     *
     * @param candidate identifier such as {@code c1}
     * @return the request
     */
    public static Request accept(String candidate) {
        return new Request("accept", candidate);
    }

    /**
     * Notation variant of the program text; the server rejects a variant
     * given with another input as the language error {@code E_ARG}.
     *
     * @param variant the variant
     * @return this request
     */
    public Request variant(Variant variant) {
        this.variant = Objects.requireNonNull(variant, "variant");
        return this;
    }

    /**
     * Sets an option by its name in the envelope, for options without a
     * method of their own, such as options of a later minor version.
     *
     * @param name option name, for example {@code diff_context}
     * @param value value as parsed JSON
     * @return this request
     */
    public Request option(String name, Object value) {
        options.put(Objects.requireNonNull(name, "name"), Objects.requireNonNull(value, name));
        return this;
    }

    /**
     * Only compute targets and effect; change nothing.
     *
     * @param preview whether to preview
     * @return this request
     */
    public Request preview(boolean preview) {
        return option("preview", preview);
    }

    /**
     * Absolute directory against which relative paths are resolved.
     *
     * @param base the directory
     * @return this request
     */
    public Request base(String base) {
        return option("base", base);
    }

    /**
     * Number of records or changes per page, from 1 to 5000.
     *
     * @param limit the number
     * @return this request
     */
    public Request limit(int limit) {
        return option("limit", (long) limit);
    }

    /**
     * Fields of the result to keep.
     *
     * @param fields names of the fields
     * @return this request
     */
    public Request fields(List<String> fields) {
        return option("fields", new ArrayList<Object>(fields));
    }

    /**
     * Format of the rendered result.
     *
     * @param format the format
     * @return this request
     */
    public Request format(Format format) {
        return option("format", format.wire());
    }

    /**
     * Level of detail of the report.
     *
     * @param verbosity the level
     * @return this request
     */
    public Request verbosity(Verbosity verbosity) {
        return option("verbosity", verbosity.wire());
    }

    /**
     * Form of differences.
     *
     * @param diff the form
     * @return this request
     */
    public Request diff(DiffMode diff) {
        return option("diff", diff.wire());
    }

    /**
     * Context lines of differences.
     *
     * @param lines number of lines
     * @return this request
     */
    public Request diffContext(int lines) {
        return option("diff_context", (long) lines);
    }

    /**
     * Largest number of characters of a single text field; {@code 0} means no limit.
     *
     * @param maxChars the number
     * @return this request
     */
    public Request maxChars(long maxChars) {
        return option("max_chars", maxChars);
    }

    /**
     * Absolute path of a file that receives the full result.
     *
     * @param path the path
     * @return this request
     */
    public Request saveTo(String path) {
        return option("save_to", path);
    }

    /**
     * Time limit of the whole program, sent in whole seconds or, when the
     * duration has a fractional second, in milliseconds.
     *
     * @param timeout the limit
     * @return this request
     */
    public Request timeout(Duration timeout) {
        String text = timeout.getNano() == 0
                ? timeout.getSeconds() + "s"
                : timeout.toMillis() + "ms";
        return option("timeout", text);
    }

    /**
     * Session identifier recorded in the journal.
     *
     * @param session the identifier
     * @return this request
     */
    public Request session(String session) {
        return option("session", session);
    }

    /**
     * Language of messages and help.
     *
     * @param language the language
     * @return this request
     */
    public Request language(Language language) {
        return option("language", language.wire());
    }

    /**
     * Script mode: several pipelines, each its own operation.
     *
     * @param script whether to use script mode
     * @return this request
     */
    public Request script(boolean script) {
        return option("script", script);
    }

    /**
     * The envelope as a JSON object.
     *
     * @return a new mutable map
     */
    public Map<String, Object> toMap() {
        Map<String, Object> envelope = new LinkedHashMap<>();
        envelope.put("nurt", ENVELOPE_VERSION);
        envelope.put(inputName, input);
        if (variant != null) {
            envelope.put("variant", variant.wire());
        }
        if (!options.isEmpty()) {
            envelope.put("options", new LinkedHashMap<>(options));
        }
        return envelope;
    }

    /**
     * The envelope as JSON text.
     *
     * @return JSON text
     */
    public String toJson() {
        return Json.write(toMap());
    }

    @Override
    public String toString() {
        return toJson();
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
