// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
package pl.danacogroup.nurt;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assertions.fail;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.Flow;
import java.util.concurrent.TimeUnit;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import java.util.stream.Stream;
import org.junit.jupiter.api.Assumptions;
import org.junit.jupiter.api.DynamicTest;
import org.junit.jupiter.api.TestFactory;
import pl.danacogroup.nurt.internal.Json;

/**
 * Runs the language-neutral binding cases {@code nurt-binding-cases/1} of
 * {@code conformance/bindings/} against the process backend.
 *
 * <p>The server is the fake server of the conformance suite: the variable
 * {@code NURT_CONFORMANCE} names the executable {@code nurt-conformance}, and
 * each case runs {@code nurt-conformance fake-serve} restricted to that case.
 * Without the variable or the case directory the cases are skipped.
 */
class BindingConformanceTest {
    private static final Duration WAIT = Duration.ofSeconds(60);
    private static final Pattern PLACEHOLDER = Pattern.compile("\\$\\{([^}]+)}");

    @TestFactory
    Stream<DynamicTest> bindingCases() throws IOException {
        String executable = System.getenv("NURT_CONFORMANCE");
        Path directory = Path.of(System.getProperty("nurt.conformance.bindings", "../../../conformance/bindings"));
        Assumptions.assumeTrue(executable != null && !executable.isEmpty(), "NURT_CONFORMANCE is not set");
        Assumptions.assumeTrue(Files.isDirectory(directory), "no binding cases in " + directory);
        List<DynamicTest> tests = new ArrayList<>();
        try (Stream<Path> files = Files.list(directory)) {
            for (Path file : files.filter(path -> path.toString().endsWith(".json")).sorted().toList()) {
                Object document = Json.parse(Files.readString(file, StandardCharsets.UTF_8));
                assertEquals("nurt-binding-cases/1", Json.string(document, "format"), file.toString());
                for (Object item : (List<?>) Json.member(document, "cases")) {
                    Map<String, Object> testCase = Json.object(item);
                    String id = Json.string(testCase, "id");
                    tests.add(DynamicTest.dynamicTest(id,
                            () -> runCase(executable, directory.getParent(), testCase)));
                }
            }
        }
        return tests.stream();
    }

    private static void runCase(String executable, Path suite, Map<String, Object> testCase) throws Exception {
        String className = Json.string(testCase, "class");
        Assumptions.assumeTrue(className == null || className.equals("mandatory"), "optional case");
        if (testCase.get("backends") instanceof List<?> backends && !backends.isEmpty()) {
            Assumptions.assumeTrue(backends.contains("process"), "case for another backend");
        }
        Path base = Files.createTempDirectory("nurt-binding-");
        createFiles(base, testCase.get("fs"));
        ClientOptions options = new ClientOptions()
                .backend(BackendKind.PROCESS)
                .command(List.of(executable, "fake-serve"))
                .environment(Map.of(
                        "NURT_CONFORMANCE_CASES", suite.toString(),
                        "NURT_CONFORMANCE_CASE", Json.string(testCase, "id")))
                .requestTimeout(WAIT);
        List<Object> views = new ArrayList<>();
        Map<Integer, Execution> executions = new LinkedHashMap<>();
        Client client = Client.create(options);
        try {
            int number = 0;
            for (Object item : (List<?>) testCase.get("steps")) {
                number++;
                Map<String, Object> step = Json.object(item);
                Map<String, Object> args = Json.object(substitute(step.getOrDefault("args", Map.of()), views, base));
                Map<String, Object> expect = Json.object(step.get("expect"));
                Object view = perform(client, executions, number, step, args, expect);
                views.add(view);
            }
        } finally {
            executions.values().forEach(Execution::close);
            client.close();
        }
    }

    private static Object perform(Client client, Map<Integer, Execution> executions, int number,
            Map<String, Object> step, Map<String, Object> args, Map<String, Object> expect) throws Exception {
        String call = Json.string(step, "call");
        String label = "step " + number + " (" + call + ")";
        String outcome;
        Map<String, Object> view = new LinkedHashMap<>();
        String errorCode = null;
        String status = null;
        try {
            Object result = invoke(client, executions, number, call, args, step);
            if (result instanceof Response response) {
                if (Boolean.TRUE.equals(args.get("raise_on_error"))) {
                    response.orThrow();
                }
                view.put("ok", response.ok());
                view.put("report", response.report());
                response.text().ifPresent(text -> view.put("text", text));
                response.error().ifPresent(error -> view.put("error", error.value()));
                response.nextCursor().ifPresent(cursor -> view.put("next_cursor", cursor));
                outcome = response.ok() ? "ok" : "language_error";
                errorCode = response.error().map(LanguageError::code).orElse(null);
            } else {
                if (result != null) {
                    view.put("value", result);
                }
                outcome = "ok";
            }
        } catch (LanguageException e) {
            outcome = "language_error";
            errorCode = e.error().code();
            view.put("error", e.error().value());
        } catch (CallException e) {
            outcome = "call_error";
            status = e.status().cName();
            view.put("status", status);
            view.put("message", e.getMessage());
        }
        assertEquals(Json.string(expect, "outcome"), outcome, label);
        if (expect.get("status") != null) {
            assertEquals(expect.get("status"), status, label);
        }
        if (expect.get("code") != null) {
            assertEquals(expect.get("code"), errorCode, label);
        }
        Map<String, Object> fields = Json.object(expect.getOrDefault("fields", Map.of()));
        for (Map.Entry<String, Object> field : fields.entrySet()) {
            Object actual = pointer(view, field.getKey());
            assertTrue(sameJson(field.getValue(), actual), label + ": " + field.getKey() + " is " + Json.write(actual));
        }
        for (Object present : (List<?>) expect.getOrDefault("present", List.of())) {
            assertTrue(exists(view, (String) present), label + ": " + present + " is missing");
        }
        for (Object absent : (List<?>) expect.getOrDefault("absent", List.of())) {
            assertFalse(exists(view, (String) absent), label + ": " + absent + " is present");
        }
        return view;
    }

    private static Object invoke(Client client, Map<Integer, Execution> executions, int number, String call,
            Map<String, Object> args, Map<String, Object> step) throws Exception {
        return switch (call) {
            case "run" -> client.run(request(args));
            case "start" -> {
                executions.put(number, client.start(request(args)));
                yield null;
            }
            case "wait" -> execution(executions, step).await(WAIT);
            case "cancel" -> {
                execution(executions, step).cancel();
                yield null;
            }
            case "events" -> events(execution(executions, step));
            case "check" -> client.check(request(args));
            case "parse" -> client.parse(text(args, "program"), variant(args));
            case "correct" -> client.correct(text(args, "program"), variant(args));
            case "print" -> client.print(Json.object(args.get("ast")), variant(args));
            case "page" -> args.get("limit") instanceof Long limit
                    ? client.page(text(args, "cursor"), limit.intValue())
                    : client.page(text(args, "cursor"));
            case "describe" -> "group".equals(args.get("layer"))
                    ? client.describe(text(args, "name"), language(args))
                    : client.describe();
            case "help" -> args.get("topic") == null ? client.help() : client.help(text(args, "topic"));
            case "catalog" -> client.catalog(withoutLocal(args));
            case "schema" -> client.schema(text(args, "which"));
            case "history" -> client.history(withoutLocal(args));
            case "space" -> args.get("volume") == null ? client.space() : client.space(text(args, "volume"));
            case "info" -> client.info();
            case "close" -> {
                client.close();
                yield null;
            }
            default -> fail("unknown call " + call);
        };
    }

    private static Execution execution(Map<Integer, Execution> executions, Map<String, Object> step) {
        Execution execution = executions.get(((Long) step.get("execution")).intValue());
        assertNotNull(execution, "execution " + step.get("execution"));
        return execution;
    }

    private static List<Object> events(Execution execution) throws Exception {
        List<Object> events = new CopyOnWriteArrayList<>();
        CompletableFuture<Void> done = new CompletableFuture<>();
        execution.events().subscribe(new Flow.Subscriber<Event>() {
            @Override
            public void onSubscribe(Flow.Subscription subscription) {
                subscription.request(Long.MAX_VALUE);
            }

            @Override
            public void onNext(Event event) {
                events.add(event.value());
            }

            @Override
            public void onError(Throwable error) {
                done.complete(null);
            }

            @Override
            public void onComplete() {
                done.complete(null);
            }
        });
        done.get(WAIT.toSeconds(), TimeUnit.SECONDS);
        return new ArrayList<>(events);
    }

    private static Request request(Map<String, Object> args) {
        Request request;
        if (args.containsKey("ast")) {
            request = Request.ast(Json.object(args.get("ast")));
        } else if (args.containsKey("cursor")) {
            request = Request.cursor(text(args, "cursor"));
        } else if (args.containsKey("accept")) {
            request = Request.accept(text(args, "accept"));
        } else {
            request = Request.program(text(args, "program"));
        }
        if (args.get("variant") instanceof String variant) {
            request.variant(Request.Variant.valueOf(variant.toUpperCase(java.util.Locale.ROOT)));
        }
        Map<String, Object> options = Json.object(args.get("options"));
        if (options != null) {
            options.forEach(request::option);
        }
        return request;
    }

    private static Map<String, Object> withoutLocal(Map<String, Object> args) {
        Map<String, Object> params = new LinkedHashMap<>(args);
        params.remove("raise_on_error");
        return params;
    }

    private static String text(Map<String, Object> args, String name) {
        return (String) args.get(name);
    }

    private static Request.Variant variant(Map<String, Object> args) {
        return args.get("variant") instanceof String variant
                ? Request.Variant.valueOf(variant.toUpperCase(java.util.Locale.ROOT))
                : Request.Variant.A;
    }

    private static Request.Language language(Map<String, Object> args) {
        return args.get("language") instanceof String language
                ? Request.Language.valueOf(language.toUpperCase(java.util.Locale.ROOT))
                : null;
    }

    private static void createFiles(Path base, Object entries) throws IOException {
        if (!(entries instanceof List<?> list)) {
            return;
        }
        for (Object item : list) {
            Map<String, Object> entry = Json.object(item);
            Path path = base.resolve(Json.string(entry, "path"));
            if ("dir".equals(entry.get("type"))) {
                Files.createDirectories(path);
            } else {
                Files.createDirectories(path.getParent());
                String content = Json.string(entry, "content_utf8");
                Files.writeString(path, content == null ? "" : content, StandardCharsets.UTF_8);
            }
        }
    }

    /** Replaces {@code ${base}} and {@code ${stepN/pointer}} in every string. */
    private static Object substitute(Object value, List<Object> views, Path base) {
        if (value instanceof String text) {
            Matcher whole = PLACEHOLDER.matcher(text);
            if (whole.matches()) {
                return resolve(whole.group(1), views, base);
            }
            Matcher matcher = PLACEHOLDER.matcher(text);
            StringBuilder out = new StringBuilder();
            while (matcher.find()) {
                Object resolved = resolve(matcher.group(1), views, base);
                matcher.appendReplacement(out, Matcher.quoteReplacement(
                        resolved instanceof String string ? string : Json.write(resolved)));
            }
            matcher.appendTail(out);
            return out.toString();
        }
        if (value instanceof Map<?, ?> map) {
            Map<String, Object> copy = new LinkedHashMap<>();
            map.forEach((name, item) -> copy.put((String) name, substitute(item, views, base)));
            return copy;
        }
        if (value instanceof List<?> list) {
            List<Object> copy = new ArrayList<>();
            list.forEach(item -> copy.add(substitute(item, views, base)));
            return copy;
        }
        return value;
    }

    private static Object resolve(String name, List<Object> views, Path base) {
        if (name.equals("base")) {
            return base.toString();
        }
        int slash = name.indexOf('/');
        String stepName = slash < 0 ? name : name.substring(0, slash);
        assertTrue(stepName.startsWith("step"), "unknown placeholder " + name);
        int step = Integer.parseInt(stepName.substring(4));
        assertTrue(step >= 1 && step <= views.size(), "placeholder " + name + " names no earlier step");
        return pointer(views.get(step - 1), slash < 0 ? "" : name.substring(slash));
    }

    /** Value at a JSON pointer (RFC 6901), or {@code null} when absent. */
    private static Object pointer(Object root, String pointer) {
        Object current = root;
        if (pointer.isEmpty()) {
            return current;
        }
        for (String token : pointer.substring(1).split("/", -1)) {
            String name = token.replace("~1", "/").replace("~0", "~");
            if (current instanceof Map<?, ?> map) {
                current = map.get(name);
            } else if (current instanceof List<?> list && name.matches("\\d+") && Integer.parseInt(name) < list.size()) {
                current = list.get(Integer.parseInt(name));
            } else {
                return null;
            }
        }
        return current;
    }

    private static boolean exists(Object root, String pointer) {
        if (pointer.isEmpty()) {
            return true;
        }
        int slash = pointer.lastIndexOf('/');
        Object parent = pointer(root, pointer.substring(0, slash));
        String name = pointer.substring(slash + 1).replace("~1", "/").replace("~0", "~");
        if (parent instanceof Map<?, ?> map) {
            return map.containsKey(name);
        }
        return parent instanceof List<?> list && name.matches("\\d+") && Integer.parseInt(name) < list.size();
    }

    private static boolean sameJson(Object expected, Object actual) {
        if (expected instanceof Number left && actual instanceof Number right) {
            return Double.compare(left.doubleValue(), right.doubleValue()) == 0;
        }
        return Json.write(expected).equals(Json.write(actual)) || (expected != null && expected.equals(actual));
    }
}
