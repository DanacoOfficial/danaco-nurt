# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Behaviour of the API, run on both backends against the mocks."""

from __future__ import annotations

import json
import os
import shutil
import threading
import time
import unittest
import warnings
from typing import Any, Dict, List

import support

from nurt import (
    Nurt,
    NurtError,
    NurtLanguageError,
    NurtTimeoutError,
    Response,
)

THREADS = 8
CALLS_PER_THREAD = 100
LONG_SLEEP = "sleep 5000"
SHORT_WAIT = 0.05


class BackendContract:
    """Tests shared by the backends; subclasses provide :meth:`make`."""

    backend = ""

    def make(self, **arguments: Any) -> Nurt:
        raise NotImplementedError

    def setUp(self) -> None:
        self.nurt = self.make()

    def tearDown(self) -> None:
        self.nurt.close()

    def test_reports_backend_in_use(self) -> None:
        self.assertEqual(self.nurt.backend, self.backend)

    def test_run_returns_response_with_rendered_text(self) -> None:
        response = self.nurt.run("list here")
        self.assertIsInstance(response, Response)
        self.assertTrue(response.ok)
        self.assertIsNone(response.error)
        self.assertEqual(response.text, "list here")
        self.assertEqual(response.report["status"], "ok")
        self.assertEqual(response.raw["nurt"], "1.0")

    def test_language_error_is_returned_not_raised(self) -> None:
        response = self.nurt.run("fail")
        self.assertFalse(response.ok)
        assert response.error is not None
        self.assertEqual(response.error["code"], "E_SYNTAX")
        self.assertEqual(response.error["fix"], ["complete the program"])

    def test_raise_on_error_turns_language_error_into_exception(self) -> None:
        with self.assertRaises(NurtLanguageError) as caught:
            self.nurt.run("fail", raise_on_error=True)
        self.assertEqual(caught.exception.code, "E_SYNTAX")
        self.assertFalse(caught.exception.response.ok)

    def test_call_failure_raises_error_with_status_name(self) -> None:
        for code, name in ((3, "NURT_E_ARG"), (5, "NURT_E_IO"), (6, "NURT_E_INTERNAL")):
            with self.subTest(status=name):
                with self.assertRaises(NurtError) as caught:
                    self.nurt.run(f"status {code}")
                self.assertEqual(caught.exception.status, name)
                self.assertEqual(caught.exception.code, code)
                self.assertIn("failure requested by the program", str(caught.exception))

    def test_unknown_status_is_reported_as_unknown(self) -> None:
        with self.assertRaises(NurtError) as caught:
            self.nurt.run("status 9")
        self.assertEqual(caught.exception.status, "NURT_E_UNKNOWN")
        self.assertIn("status 9", str(caught.exception))

    def test_panic_at_boundary_is_internal_error(self) -> None:
        with self.assertRaises(NurtError) as caught:
            self.nurt.run("panic")
        self.assertEqual(caught.exception.status, "NURT_E_INTERNAL")

    def test_explicit_option_overrides_default(self) -> None:
        response = self.nurt.run("options", language="pl")
        self.assertEqual(json.loads(response.text)["language"], "pl")

    def test_unknown_option_is_rejected_before_the_call(self) -> None:
        with self.assertRaises(TypeError):
            self.nurt.run("list", colour=True)

    def test_page_follows_next_cursor(self) -> None:
        first = self.nurt.run("page")
        self.assertEqual(first.next_cursor, "page-2")
        assert first.next_cursor is not None
        second = self.nurt.page(first.next_cursor)
        self.assertEqual(second.text, "page 2")
        self.assertIsNone(second.next_cursor)

    def test_expired_cursor_is_language_error(self) -> None:
        self.assertFalse(self.nurt.page("page-0").ok)
        with self.assertRaises(NurtLanguageError):
            self.nurt.page("page-0", raise_on_error=True)

    def test_parse_returns_syntax_tree(self) -> None:
        result = self.nurt.parse("list here", variant="b")
        self.assertTrue(result.ok)
        assert result.ast is not None
        self.assertEqual(result.ast["program"], "list here")
        self.assertEqual(result.diagnostics, [])

    def test_parse_failure_has_diagnostics_and_no_tree(self) -> None:
        result = self.nurt.parse("fail")
        self.assertFalse(result.ok)
        self.assertEqual(result.diagnostics[0]["severity"], "error")

    def test_print_accepts_tree_as_object_or_json(self) -> None:
        tree = self.nurt.parse("list here").ast
        self.assertEqual(self.nurt.print(tree), "list here")
        self.assertEqual(self.nurt.print(json.dumps(tree), variant="c"), "list here")

    def test_unknown_variant_is_argument_error(self) -> None:
        with self.assertRaises(NurtError) as caught:
            self.nurt.parse("list", variant="d")
        self.assertEqual(caught.exception.status, "NURT_E_ARG")

    def test_correct_returns_program_and_corrections(self) -> None:
        result = self.nurt.correct("list here")
        self.assertEqual(result.program, "list here")
        self.assertEqual(result.corrections, [])

    def test_check_returns_bound_tree(self) -> None:
        result = self.nurt.check("list here")
        self.assertTrue(result.ok)
        assert result.bound is not None
        self.assertEqual(result.bound["program"], "list here")
        self.assertEqual(result.raw["effects"], [])
        self.assertEqual(result.diagnostics, [])
        self.assertIsNone(result.error)

    def test_check_failure_has_diagnostics(self) -> None:
        result = self.nurt.check("fail")
        self.assertFalse(result.ok)
        self.assertIsNone(result.bound)
        self.assertEqual(len(result.diagnostics), 1)

    def test_text_queries(self) -> None:
        self.assertEqual(self.nurt.help("journal"), "help journal")
        self.assertEqual(self.nurt.help(), "help index")
        description = self.nurt.describe("group", name="documents", language="pl")
        self.assertTrue(description.startswith("description "))
        self.assertEqual(
            json.loads(description[len("description ") :]),
            {"layer": "group", "language": "pl", "name": "documents"},
        )

    def test_json_queries(self) -> None:
        self.assertEqual(self.nurt.catalog()["catalog_rev"], "mock")
        self.assertEqual(self.nurt.catalog({"name": "find"})["catalog_rev"], "mock")
        self.assertEqual(self.nurt.schema("report"), {"title": "nurt-report/1"})
        self.assertEqual(self.nurt.history({"limit": 1}), {"entries": []})
        self.assertEqual(self.nurt.history("last 5"), {"entries": []})
        self.assertEqual(self.nurt.space("/home/user"), {"volumes": []})
        self.assertEqual(self.nurt.info()["implementation"], "mock")

    def test_unknown_schema_is_argument_error(self) -> None:
        with self.assertRaises(NurtError) as caught:
            self.nurt.schema("unknown")
        self.assertEqual(caught.exception.status, "NURT_E_ARG")

    def test_execution_events_are_iterated_in_order(self) -> None:
        with self.nurt.start("events 3") as execution:
            events = list(execution)
            response = execution.wait()
        self.assertEqual([e["t"] for e in events], ["job.progress"] * 3 + ["exec.finished"])
        self.assertEqual([e["seq"] for e in events], [1, 2, 3, 4])
        self.assertEqual(response.text, "events 3")

    def test_on_event_runs_in_background_thread(self) -> None:
        received: List[Dict[str, Any]] = []
        threads = set()

        def on_event(event: Dict[str, Any]) -> None:
            received.append(event)
            threads.add(threading.get_ident())

        with self.nurt.start("events 2", on_event=on_event) as execution:
            execution.wait()
            with self.assertRaises(TypeError):
                iter(execution).__next__()
        self.assertEqual([e["seq"] for e in received], [1, 2, 3])
        self.assertNotIn(threading.get_ident(), threads)

    def test_exception_in_on_event_is_raised_by_wait(self) -> None:
        def on_event(event: Dict[str, Any]) -> None:
            raise ValueError("rejected event")

        with self.nurt.start("events 1", on_event=on_event) as execution:
            with self.assertRaises(ValueError):
                execution.wait()

    def test_wait_timeout_then_cancel(self) -> None:
        with self.nurt.start(LONG_SLEEP) as execution:
            with self.assertRaises(NurtTimeoutError) as caught:
                execution.wait(SHORT_WAIT)
            self.assertIsInstance(caught.exception, TimeoutError)
            self.assertEqual(caught.exception.status, "NURT_E_TIMEOUT")
            started = time.monotonic()
            execution.cancel()
            execution.cancel()
            response = execution.wait()
            self.assertLess(time.monotonic() - started, 4)
        self.assertIsInstance(response, Response)
        self.assertFalse(response.ok)
        assert response.error is not None
        self.assertEqual(response.error["code"], "E_CANCELLED")

    def test_cancelled_execution_raises_with_raise_on_error(self) -> None:
        with self.nurt.start(LONG_SLEEP, raise_on_error=True) as execution:
            execution.cancel()
            with self.assertRaises(NurtLanguageError) as caught:
                execution.wait()
        self.assertEqual(caught.exception.code, "E_CANCELLED")

    def test_cancel_after_completion_has_no_effect(self) -> None:
        with self.nurt.start("done") as execution:
            self.assertEqual(execution.wait().text, "done")
            execution.cancel()
            self.assertEqual(execution.wait().text, "done")

    def test_raise_on_error_applies_to_execution(self) -> None:
        with self.nurt.start("fail", raise_on_error=True) as execution:
            with self.assertRaises(NurtLanguageError):
                execution.wait()

    def test_parallel_calls_from_many_threads(self) -> None:
        failures: List[str] = []

        def worker(index: int) -> None:
            for call in range(CALLS_PER_THREAD):
                program = f"echo {index} {call}"
                try:
                    text = self.nurt.run(program).text
                except NurtError as exc:
                    failures.append(str(exc))
                    return
                if text != program:
                    failures.append(f"{program!r} returned {text!r}")

        threads = [threading.Thread(target=worker, args=(i,)) for i in range(THREADS)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        self.assertEqual(failures, [])

    def test_calls_after_close_fail(self) -> None:
        self.nurt.close()
        self.nurt.close()
        with self.assertRaises(NurtError) as caught:
            self.nurt.run("list")
        self.assertEqual(caught.exception.status, "NURT_E_ARG")

    @support.requires_fork
    def test_child_process_after_fork_gets_working_engine(self) -> None:
        self.assertEqual(self.nurt.run("parent").text, "parent")
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            pid = os.fork()
        if pid == 0:
            code = 1
            try:
                code = 0 if self.nurt.run("child").text == "child" else 1
            finally:
                os._exit(code)
        self.assertEqual(support.wait_child(pid), 0)
        self.assertEqual(self.nurt.run("parent again").text, "parent again")


class ProcessBackendTest(BackendContract, unittest.TestCase):
    backend = "process"

    def make(self, **arguments: Any) -> Nurt:
        return Nurt(backend="process", executable=support.mock_executable(), **arguments)

    def test_session_and_language_are_sent_in_initialize(self) -> None:
        with self.make(session="s-0001", language="pl") as nurt:
            options = nurt.run("options", limit=5, preview=True, format=None)
            defaults = nurt.run("session")
        self.assertEqual(json.loads(options.text), {"limit": 5, "preview": True})
        self.assertEqual(json.loads(defaults.text), {"session": "s-0001", "language": "pl"})

    def test_child_exit_fails_call_and_next_call_restarts(self) -> None:
        with self.assertRaises(NurtError) as caught:
            self.nurt.run("crash")
        self.assertEqual(caught.exception.status, "NURT_E_INTERNAL")
        self.assertIn("exited with code 3", caught.exception.message)
        self.assertIn("crash requested", caught.exception.message)
        self.assertEqual(self.nurt.run("after restart").text, "after restart")

    def test_child_exit_fails_running_execution(self) -> None:
        execution = self.nurt.start(LONG_SLEEP)
        with self.assertRaises(NurtError):
            self.nurt.run("crash")
        with self.assertRaises(NurtError) as caught:
            execution.wait(5)
        self.assertEqual(caught.exception.status, "NURT_E_INTERNAL")
        self.assertEqual(list(execution), [])
        execution.close()

    def test_protocol_mismatch_is_abi_error(self) -> None:
        os.environ["NURT_MOCK_PROTOCOL"] = "nurt-rpc/2"
        try:
            with self.make() as nurt:
                with self.assertRaises(NurtError) as caught:
                    nurt.info()
        finally:
            del os.environ["NURT_MOCK_PROTOCOL"]
        self.assertEqual(caught.exception.status, "NURT_E_ABI")

    def test_missing_executable_lists_checked_location(self) -> None:
        missing = str(support.scratch() / "absent" / "nurt")
        with self.assertRaises(NurtError) as caught:
            Nurt(backend="process", executable=missing)
        self.assertEqual(caught.exception.status, "NURT_E_IO")
        self.assertIn(missing, caught.exception.message)


class LibraryBackendTest(BackendContract, unittest.TestCase):
    backend = "library"

    def make(self, **arguments: Any) -> Nurt:
        return Nurt(backend="library", library=support.mock_library(), **arguments)

    def test_session_and_language_are_request_options(self) -> None:
        with self.make(session="s-0001", language="pl") as nurt:
            response = nurt.run("options", limit=5, preview=True, format=None)
            description = nurt.describe()
        self.assertEqual(
            json.loads(response.text),
            {"limit": 5, "preview": True, "session": "s-0001", "language": "pl"},
        )
        self.assertEqual(json.loads(description[len("description ") :])["language"], "pl")

    def test_buffers_are_released_after_every_call(self) -> None:
        controls = support.mock_controls(support.mock_library())
        self.nurt.run("list")
        self.nurt.run("fail")
        self.nurt.parse("list")
        self.nurt.check("fail")
        for program in ("status 3", "panic"):
            with self.assertRaises(NurtError):
                self.nurt.run(program)
        with self.nurt.start("events 5") as execution:
            execution.wait()
        self.assertEqual(controls.live_buffers(), 0)

    def test_executions_and_engines_are_released(self) -> None:
        controls = support.mock_controls(support.mock_library())
        executions, engines = controls.live_executions(), controls.live_engines()
        with self.make() as nurt:
            self.assertEqual(controls.live_engines(), engines + 1)
            finished = nurt.start("done")
            finished.wait()
            running = nurt.start(LONG_SLEEP)
            self.assertEqual(controls.live_executions(), executions + 2)
            finished.close()
            self.assertEqual(controls.live_executions(), executions + 1)
        self.assertEqual(controls.live_executions(), executions)
        self.assertEqual(controls.live_engines(), engines)
        with self.assertRaises(NurtError):
            running.wait(0)

    def test_calls_release_the_gil(self) -> None:
        sleepers = 4
        sleep_ms = 300
        started = time.monotonic()
        threads = [threading.Thread(target=self.nurt.run, args=(f"sleep {sleep_ms}",)) for _ in range(sleepers)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        elapsed = time.monotonic() - started
        self.assertLess(elapsed, sleepers * sleep_ms / 1000 * 0.75)

    def test_incompatible_interface_version_is_abi_error(self) -> None:
        copy = support.scratch() / "abi" / support.library_file_name()
        copy.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(support.mock_library(), copy)
        support.mock_controls(support.mock_library())
        os.environ["NURT_MOCK_ABI"] = "2.0"
        try:
            with self.assertRaises(NurtError) as caught:
                Nurt(backend="library", library=str(copy))
        finally:
            del os.environ["NURT_MOCK_ABI"]
        self.assertEqual(caught.exception.status, "NURT_E_ABI")
        self.assertIn("2.0", caught.exception.message)

    def test_log_messages_reach_logging(self) -> None:
        with self.assertLogs("nurt", level="INFO") as logs:
            self.make().close()
        self.assertIn("engine opened", logs.output[0])

    def test_engine_open_failure_reports_last_error(self) -> None:
        controls = support.mock_controls(support.mock_library())
        controls.fail_next("nurt_engine_open", 5)
        with self.assertRaises(NurtError) as caught:
            self.make()
        self.assertEqual(caught.exception.status, "NURT_E_IO")
        self.assertEqual(caught.exception.message, "forced failure")

    def test_configuration_must_be_an_object(self) -> None:
        with self.assertRaises(NurtError) as caught:
            self.make(config="[1]")
        self.assertEqual(caught.exception.status, "NURT_E_ARG")

    def test_configuration_is_passed_as_json_object(self) -> None:
        with self.make(config={"output": {"language": "en"}}) as nurt:
            self.assertEqual(nurt.run("list").text, "list")


class AutoBackendTest(unittest.TestCase):
    def test_auto_prefers_library(self) -> None:
        with Nurt(library=support.mock_library(), executable=support.mock_executable()) as nurt:
            self.assertEqual(nurt.backend, "library")

    def test_auto_falls_back_to_process(self) -> None:
        missing = str(support.scratch() / "absent" / support.library_file_name())
        with Nurt(library=missing, executable=support.mock_executable()) as nurt:
            self.assertEqual(nurt.backend, "process")
            self.assertEqual(nurt.run("list").text, "list")

    def test_auto_reports_both_failures(self) -> None:
        absent = support.scratch() / "absent"
        with self.assertRaises(NurtError) as caught:
            Nurt(library=str(absent / "lib"), executable=str(absent / "nurt"))
        self.assertEqual(caught.exception.status, "NURT_E_IO")
        self.assertIn("library backend", caught.exception.message)

    def test_unknown_backend_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            Nurt(backend="remote")


if __name__ == "__main__":
    unittest.main()
