#!/usr/bin/env python3
#
# Copyright (c) 2026-present, the Ladybird developers.
#
# SPDX-License-Identifier: BSD-2-Clause

# A Compositor that dies while WebContent waits in a synchronous WebGL call must not take WebContent down with it. The
# call fails, the page's WebGL context is lost, and once the browser has connected a new Compositor, the page can create
# a working WebGL context again.

import argparse
import importlib
import os
import signal
import subprocess
import tempfile
import threading
import time
import urllib.parse

from pathlib import Path

webdriver_helpers = importlib.import_module("test-webdriver-delete-session")
render_clock_reconnect = importlib.import_module("test-webdriver-render-clock-reconnect")

PAGE = "data:text/html," + urllib.parse.quote(
    """<canvas id=canvas width=16 height=16></canvas><script>
window.gl = canvas.getContext("webgl");
</script>"""
)

# Every getError() is a synchronous call to the Compositor. The loop ends once a failed call has lost the context.
SYNC_CALLS_UNTIL_CONTEXT_LOST = """
let calls = 0;
while (!gl.isContextLost()) {
    gl.getError();
    ++calls;
}
return calls;
"""

# Resolves once a new WebGL context works, which it does once WebContent has connected to the new Compositor.
WAIT_FOR_WORKING_CONTEXT = """
const done = arguments[0];
(function poll() {
    const context = document.createElement("canvas").getContext("webgl");
    if (context !== null && !context.isContextLost() && context.getError() === context.NO_ERROR)
        done(true);
    else
        setTimeout(poll, 10);
})();
"""


def execute(port, session, script, mode="sync"):
    status, payload, body = webdriver_helpers.request(
        port, "POST", f"/session/{session}/execute/{mode}", {"script": script, "args": []}
    )
    assert status == 200, body
    return payload["value"]


def main_thread_is_waiting_on_futex(pid):
    try:
        return "futex" in Path(f"/proc/{pid}/task/{pid}/wchan").read_text()
    except OSError:
        return None


def wait_until_blocked_in_sync_call(web_content):
    # With the Compositor stopped, the first synchronous call leaves WebContent's main thread waiting for an answer
    # for good. Where the kernel tells us what the thread waits on, wait until it has settled there.
    deadline = time.monotonic() + webdriver_helpers.EVENT_TIMEOUT_SECONDS
    consecutive_samples = 0
    while consecutive_samples < 5:
        waiting = main_thread_is_waiting_on_futex(web_content)
        if waiting is None:
            return
        consecutive_samples = consecutive_samples + 1 if waiting else 0
        if time.monotonic() >= deadline:
            raise AssertionError("WebContent never blocked in a synchronous call to the stopped Compositor")
        time.sleep(0.05)


def run_test(webdriver_binary):
    with tempfile.TemporaryDirectory(prefix="ladybird-compositor-loss-") as temporary:
        environment = os.environ.copy()
        for variable, directory in (
            ("XDG_DATA_HOME", "data"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_CACHE_HOME", "cache"),
        ):
            environment[variable] = str(Path(temporary) / directory)
        port = webdriver_helpers.unused_port()
        webdriver = subprocess.Popen(
            [webdriver_binary, "--headless", "-l", "127.0.0.1", "-p", str(port)],
            env=environment,
        )
        try:
            webdriver_helpers.wait_for_port(port)
            session = webdriver_helpers.create_session(port)
            status, _, body = webdriver_helpers.request(port, "POST", f"/session/{session}/url", {"url": PAGE})
            assert status == 200, body
            assert execute(port, session, "return gl !== null && gl.getError() === gl.NO_ERROR;") is True

            compositors = render_clock_reconnect.descendants_named(webdriver.pid, "Compositor")
            assert len(compositors) == 1, f"Expected one Compositor, found {len(compositors)}"
            web_contents = render_clock_reconnect.descendants_named(webdriver.pid, "WebContent")
            assert len(web_contents) == 1, f"Expected one WebContent, found {len(web_contents)}"

            os.kill(compositors[0], signal.SIGSTOP)
            result = {}

            def run_sync_calls():
                result["calls"] = execute(port, session, SYNC_CALLS_UNTIL_CONTEXT_LOST)

            sync_calls = threading.Thread(target=run_sync_calls)
            sync_calls.start()
            wait_until_blocked_in_sync_call(web_contents[0])
            os.kill(compositors[0], signal.SIGKILL)
            sync_calls.join(timeout=webdriver_helpers.EVENT_TIMEOUT_SECONDS)
            assert not sync_calls.is_alive(), "The synchronous WebGL calls never ended"
            assert isinstance(result.get("calls"), int), result

            deadline = time.monotonic() + webdriver_helpers.EVENT_TIMEOUT_SECONDS
            while True:
                replacements = [
                    pid
                    for pid in render_clock_reconnect.descendants_named(webdriver.pid, "Compositor")
                    if pid != compositors[0]
                ]
                if replacements:
                    break
                if time.monotonic() >= deadline:
                    raise AssertionError("No Compositor replaced the one that died")
                time.sleep(0.05)

            # The same WebContent carries on, and WebGL works again through the new Compositor.
            assert render_clock_reconnect.descendants_named(webdriver.pid, "WebContent") == web_contents
            assert execute(port, session, WAIT_FOR_WORKING_CONTEXT, "async") is True
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
