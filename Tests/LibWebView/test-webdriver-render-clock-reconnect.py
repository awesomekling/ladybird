#!/usr/bin/env python3
#
# Copyright (c) 2026-present, the Ladybird developers.
#
# SPDX-License-Identifier: BSD-2-Clause

# The render clock presents an idle page's animation from the render side. When the
# Compositor dies, the leases end with the render clock's channel; once the browser has connected a new Compositor, the
# next rendering update grants them anew, and the render clock presents frames again.

import argparse
import importlib
import os
import shlex
import signal
import subprocess
import tempfile
import time
import urllib.parse

from pathlib import Path

webdriver_helpers = importlib.import_module("test-webdriver-delete-session")

PAGE = "data:text/html," + urllib.parse.quote(
    """<style>
#box { width: 100px; height: 20px; background: blue; animation: grow 60s linear; }
@keyframes grow { from { width: 100px; } to { width: 2100px; } }
</style><div id=box></div>"""
)


def descendants_named(root_pid, process_name):
    rows = subprocess.check_output(["ps", "-axo", "pid=,ppid=,command="], text=True).splitlines()
    children = {}
    for row in rows:
        fields = row.strip().split(None, 2)
        pid, parent = fields[:2]
        command = fields[2] if len(fields) == 3 else ""
        children.setdefault(int(parent), []).append((int(pid), command))
    pending = [root_pid]
    found = []
    while pending:
        for pid, command in children.get(pending.pop(), []):
            pending.append(pid)
            arguments = shlex.split(command)
            if arguments and Path(arguments[0]).name == process_name:
                found.append(pid)
    return found


def execute_async(port, session, script, args):
    status, payload, body = webdriver_helpers.request(
        port, "POST", f"/session/{session}/execute/async", {"script": script, "args": args}
    )
    assert status == 200, body
    return payload["value"]


# Resolves once the render clock presented `count` more frames from the render side, however long the display takes.
WAIT_FOR_PRESENTED_FRAMES = """
const [count, done] = arguments;
const presented = () => internals.getRenderClockCounters().ticksPresented;
const target = presented() + count;
(function poll() {
    if (presented() >= target)
        done(true);
    else
        setTimeout(poll, 10);
})();
"""


def run_test(webdriver_binary):
    with tempfile.TemporaryDirectory(prefix="ladybird-render-clock-") as temporary:
        environment = os.environ.copy()
        for variable, directory in (
            ("XDG_DATA_HOME", "data"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_CACHE_HOME", "cache"),
        ):
            environment[variable] = str(Path(temporary) / directory)
        port = webdriver_helpers.unused_port()
        webdriver = subprocess.Popen(
            [webdriver_binary, "--headless", "--expose-internals-object", "-l", "127.0.0.1", "-p", str(port)],
            env=environment,
        )
        try:
            webdriver_helpers.wait_for_port(port)
            session = webdriver_helpers.create_session(port)
            status, _, body = webdriver_helpers.request(port, "POST", f"/session/{session}/url", {"url": PAGE})
            assert status == 200, body
            assert execute_async(port, session, WAIT_FOR_PRESENTED_FRAMES, [5]) is True

            compositors = descendants_named(webdriver.pid, "Compositor")
            assert len(compositors) == 1, f"Expected one Compositor, found {len(compositors)}"
            os.kill(compositors[0], signal.SIGKILL)

            deadline = time.monotonic() + webdriver_helpers.EVENT_TIMEOUT_SECONDS
            while True:
                replacements = [pid for pid in descendants_named(webdriver.pid, "Compositor") if pid != compositors[0]]
                if replacements:
                    break
                if time.monotonic() >= deadline:
                    raise AssertionError("No Compositor replaced the one that died")
                time.sleep(0.05)

            # The render clock presents frames again from the render side, through the new Compositor.
            assert execute_async(port, session, WAIT_FOR_PRESENTED_FRAMES, [5]) is True
            webdriver_helpers.request(port, "DELETE", f"/session/{session}")
        finally:
            webdriver.terminate()
            try:
                webdriver.wait(timeout=5)
            except subprocess.TimeoutExpired:
                webdriver.kill()
                webdriver.wait()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("webdriver_binary")
    args = parser.parse_args()
    run_test(args.webdriver_binary)
