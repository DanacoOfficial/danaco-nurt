# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""``AsyncNurt`` on both backends."""

from __future__ import annotations

import asyncio
import threading
import unittest
from typing import Any, Dict, List

import support

from nurt import AsyncNurt, NurtCancelledError, NurtError, NurtTimeoutError

CONCURRENT_CALLS = 20


class AsyncContract:
    def make(self) -> AsyncNurt:
        raise NotImplementedError

    async def asyncSetUp(self) -> None:
        self.nurt = self.make()

    async def asyncTearDown(self) -> None:
        await self.nurt.close()

    async def test_run_and_queries(self) -> None:
        response = await self.nurt.run("list here")
        self.assertEqual(response.text, "list here")
        self.assertTrue((await self.nurt.parse("list")).ok)
        self.assertEqual(await self.nurt.help("journal"), "help journal")
        self.assertEqual((await self.nurt.info())["implementation"], "mock")
        self.assertEqual((await self.nurt.page("page-2")).text, "page 2")
        self.assertEqual((await self.nurt.check("list")).diagnostics, [])

    async def test_concurrent_calls_do_not_block_the_loop(self) -> None:
        programs = [f"echo {i}" for i in range(CONCURRENT_CALLS)]
        responses = await asyncio.gather(*(self.nurt.run(p) for p in programs))
        self.assertEqual([r.text for r in responses], programs)

    async def test_events_are_iterated_with_async_for(self) -> None:
        async with await self.nurt.start("events 2") as execution:
            events = [event async for event in execution]
            response = await execution.wait()
        self.assertEqual([e["seq"] for e in events], [1, 2, 3])
        self.assertEqual(response.text, "events 2")

    async def test_on_event_runs_in_loop_thread(self) -> None:
        loop_thread = threading.get_ident()
        threads: List[int] = []
        received: List[Dict[str, Any]] = []

        def on_event(event: Dict[str, Any]) -> None:
            threads.append(threading.get_ident())
            received.append(event)

        async with await self.nurt.start("events 1", on_event=on_event) as execution:
            await execution.wait()
            await asyncio.sleep(0.05)
        self.assertEqual([e["t"] for e in received], ["job.progress", "exec.finished"])
        self.assertEqual(set(threads), {loop_thread})

    async def test_timeout_and_cancel(self) -> None:
        execution = await self.nurt.start("sleep 5000")
        with self.assertRaises(NurtTimeoutError):
            await execution.wait(0.05)
        await execution.cancel()
        with self.assertRaises(NurtCancelledError):
            await execution.wait()
        await execution.close()

    async def test_call_failure_raises(self) -> None:
        with self.assertRaises(NurtError) as caught:
            await self.nurt.run("status 3")
        self.assertEqual(caught.exception.status, "NURT_E_ARG")


class AsyncProcessTest(AsyncContract, unittest.IsolatedAsyncioTestCase):
    def make(self) -> AsyncNurt:
        return AsyncNurt(backend="process", executable=support.mock_executable())


class AsyncLibraryTest(AsyncContract, unittest.IsolatedAsyncioTestCase):
    def make(self) -> AsyncNurt:
        return AsyncNurt(backend="library", library=support.mock_library())


if __name__ == "__main__":
    unittest.main()
