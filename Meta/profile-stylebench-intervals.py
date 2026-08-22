#!/usr/bin/env python3

# Copyright (c) 2026-present, the Ladybird developers.
# SPDX-License-Identifier: BSD-2-Clause

"""Measure the work StyleBench performs outside its timed test bodies."""

import argparse
import functools
import http.server
import json
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request

from pathlib import Path

DEFAULT_WEBDRIVER = "Build/release/bin/WebDriver"
DEFAULT_STYLEBENCH = Path.home() / "src/web-benchmarks/benchmarks/StyleBench"

PROFILE_SCRIPT = r"""
try {
window.__styleBenchProfile = { events: [], done: false };
window.__styleBenchStamp = function (name, suite, test) {
    window.__styleBenchProfile.events.push({
        name,
        suite: suite || window.__styleBenchSuite || "",
        test: test || window.__styleBenchTest || "",
        time: performance.now(),
    });
};
window.__styleBenchDuration = function (name, suite, duration) {
    window.__styleBenchProfile.events.push({ name, suite, test: "", duration });
};

function wrapDuration(object, method, eventName) {
    const original = object[method];
    object[method] = function (...args) {
        const start = performance.now();
        const result = original.apply(this, args);
        __styleBenchDuration(eventName, window.__styleBenchSuite || "", performance.now() - start);
        return result;
    };
}

wrapDuration(BenchmarkRunner.prototype, "_removeFrame", "remove-frame");
wrapDuration(BenchmarkRunner.prototype, "_appendFrame", "append-frame");

for (const suite of Suites) {
    const originalPrepare = suite.prepare;
    suite.prepare = function (runner, contentWindow, contentDocument) {
        window.__styleBenchSuite = suite.name;
        window.__styleBenchTest = "";
        __styleBenchStamp("prepare-start", suite.name, "");

        const originalCreateBenchmark = contentWindow.createBenchmark;
        contentWindow.createBenchmark = function (configuration) {
            contentWindow.eval(`
                for (const [method, eventName] of [
                    ["makeStylesheet", "generate-stylesheet"],
                    ["makeStyle", "make-style"],
                    ["makeTree", "make-tree"],
                    ["updateCachedTestElements", "cache-elements"],
                ]) {
                    const original = StyleBench.prototype[method];
                    StyleBench.prototype[method] = function (...args) {
                        const start = parent.performance.now();
                        const result = original.apply(this, args);
                        parent.__styleBenchDuration(eventName, ${JSON.stringify(suite.name)}, parent.performance.now() - start);
                        return result;
                    };
                }
            `);
            const start = performance.now();
            const result = originalCreateBenchmark.call(this, configuration);
            __styleBenchDuration("construct-benchmark", suite.name, performance.now() - start);
            return result;
        };

        const promise = originalPrepare.call(this, runner, contentWindow, contentDocument);
        const originalResolve = promise.resolve;
        promise.resolve = function (value) {
            __styleBenchStamp("prepare-end", suite.name, "");
            return originalResolve.call(this, value);
        };
        return promise;
    };
}

const originalWriteMark = BenchmarkRunner.prototype._writeMark;
BenchmarkRunner.prototype._writeMark = function (name) {
    originalWriteMark.call(this, name);
    __styleBenchStamp("mark:" + name, window.__styleBenchSuite, window.__styleBenchTest);
};

const originalRunTest = BenchmarkRunner.prototype._runTest;
BenchmarkRunner.prototype._runTest = function (suite, test, prepareReturnValue, callback) {
    __styleBenchStamp("run-test-enter", suite.name, test.name);
    return originalRunTest.call(this, suite, test, prepareReturnValue, function (...args) {
        __styleBenchStamp("run-test-callback", suite.name, test.name);
        return callback.apply(this, args);
    });
};

const originalWillRunTest = benchmarkClient.willRunTest;
benchmarkClient.willRunTest = function (suite, test) {
    window.__styleBenchSuite = suite.name;
    window.__styleBenchTest = test.name;
    __styleBenchStamp("will-run-start", suite.name, test.name);
    const result = originalWillRunTest.call(this, suite, test);
    __styleBenchStamp("will-run-end", suite.name, test.name);
    return result;
};

const originalDidRunTest = benchmarkClient.didRunTest;
benchmarkClient.didRunTest = function (suite, test) {
    __styleBenchStamp("did-run-start", suite.name, test.name);
    const result = originalDidRunTest.call(this, suite, test);
    __styleBenchStamp("did-run-end", suite.name, test.name);
    return result;
};

const originalDidFinish = benchmarkClient.didFinishLastIteration;
benchmarkClient.didFinishLastIteration = function (...args) {
    const result = originalDidFinish.apply(this, args);
    __styleBenchStamp("finished", "", "");
    __styleBenchProfile.done = true;
    return result;
};

__styleBenchStamp("benchmark-start", "", "");
startBenchmark();
return true;
} catch (error) {
    return "PROFILE_ERROR: " + error + "\n" + error.stack;
}
"""


class QuietRequestHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, format, *args):
        pass

    def do_GET(self):
        if self.path.split("?", 1)[0].endswith("/resources/benchmark-report.js"):
            content = b"window.__styleBenchReporterSuppressed = true;\n"
            self.send_response(200)
            self.send_header("Content-Type", "text/javascript")
            self.send_header("Content-Length", str(len(content)))
            self.end_headers()
            self.wfile.write(content)
            return
        super().do_GET()


def request_json(base_url, method, path, body=None, timeout=600):
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(
        base_url + path,
        data=data,
        headers={"Content-Type": "application/json"},
        method=method,
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        raise RuntimeError(error.read().decode()) from error


def unused_port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def wait_for_webdriver(base_url, process):
    for _ in range(200):
        if process.poll() is not None:
            raise RuntimeError("WebDriver exited before accepting connections")
        try:
            request_json(base_url, "GET", "/status", timeout=1)
            return
        except (OSError, urllib.error.URLError):
            time.sleep(0.05)
    raise RuntimeError("WebDriver did not accept connections within 10 seconds")


def stop_process(process):
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()


def run_profile(arguments):
    stylebench = arguments.stylebench.resolve()
    server_port = unused_port()
    handler = functools.partial(QuietRequestHandler, directory=stylebench.parent)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", server_port), handler)
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()

    webdriver_port = unused_port()
    base_url = f"http://127.0.0.1:{webdriver_port}"
    benchmark_url = f"http://127.0.0.1:{server_port}/{stylebench.name}/index.html?iterationCount=1"

    with tempfile.TemporaryDirectory(prefix="ladybird-stylebench-intervals-") as temporary_directory:
        process = subprocess.Popen(
            [
                str(arguments.webdriver),
                "--headless",
                "--disable-sandbox",
                "--profiles-directory",
                temporary_directory,
                "--port",
                str(webdriver_port),
            ],
            stdout=subprocess.DEVNULL,
            stderr=None if arguments.keep_stderr else subprocess.DEVNULL,
        )
        session = None
        try:
            wait_for_webdriver(base_url, process)
            session = request_json(
                base_url,
                "POST",
                "/session",
                {"capabilities": {"alwaysMatch": {}}},
            )["value"]["sessionId"]
            request_json(
                base_url,
                "POST",
                f"/session/{session}/timeouts",
                {"script": 600_000, "pageLoad": 600_000},
            )
            request_json(base_url, "POST", f"/session/{session}/url", {"url": benchmark_url})
            readiness = request_json(
                base_url,
                "POST",
                f"/session/{session}/execute/sync",
                {
                    "script": "return { runner: typeof BenchmarkRunner, suites: typeof Suites, client: typeof benchmarkClient, start: typeof startBenchmark };",
                    "args": [],
                },
            )["value"]
            if readiness != {"runner": "function", "suites": "object", "client": "object", "start": "function"}:
                raise RuntimeError(f"StyleBench scripts did not initialize: {readiness}")
            start_result = request_json(
                base_url,
                "POST",
                f"/session/{session}/execute/sync",
                {"script": PROFILE_SCRIPT, "args": []},
            )["value"]
            if start_result is not True:
                raise RuntimeError(start_result)

            deadline = time.monotonic() + arguments.timeout
            while time.monotonic() < deadline:
                profile = request_json(
                    base_url,
                    "POST",
                    f"/session/{session}/execute/sync",
                    {"script": "return window.__styleBenchProfile;", "args": []},
                )["value"]
                if profile["done"]:
                    return profile
                time.sleep(0.1)
            raise RuntimeError("StyleBench profiling timed out")
        finally:
            if session is not None:
                try:
                    request_json(base_url, "DELETE", f"/session/{session}", timeout=10)
                except (OSError, urllib.error.URLError):
                    pass
            stop_process(process)
            server.shutdown()
            server.server_close()
            server_thread.join(timeout=2)


def print_profile(profile):
    durations = {}
    timestamps = {}
    named_timestamps = {}
    suite_order = []
    test_order = {}
    for event in profile["events"]:
        if "duration" in event:
            durations.setdefault((event["suite"], event["name"]), []).append(event["duration"])
            continue
        timestamps[(event["suite"], event["test"], event["name"])] = event["time"]
        named_timestamps[event["name"]] = event["time"]
        if event["name"] == "prepare-start":
            suite_order.append(event["suite"])
            test_order[event["suite"]] = []
        elif event["name"] == "will-run-start":
            test_order[event["suite"]].append(event["test"])

    def timestamp(suite, test, name):
        return timestamps[(suite, test, name)]

    def elapsed(suite, test, start, end):
        return timestamp(suite, test, end) - timestamp(suite, test, start)

    columns = (
        "prepare",
        "first-render",
        "later-timers",
        "preflight",
        "measured",
        "post-render",
        "callbacks",
    )
    rows = {}
    for suite in suite_order:
        row = dict.fromkeys(columns, 0.0)
        row["prepare"] = timestamp(suite, "", "prepare-end") - timestamp(suite, "", "prepare-start")
        for test_index, test in enumerate(test_order[suite]):
            wait = elapsed(suite, test, "will-run-end", "run-test-enter")
            if test_index == 0:
                row["first-render"] += wait
            else:
                row["later-timers"] += wait
            row["preflight"] += elapsed(suite, test, "run-test-enter", f"mark:{suite}.{test}-start")
            row["measured"] += elapsed(suite, test, f"mark:{suite}.{test}-start", f"mark:{suite}.{test}-sync-end")
            row["post-render"] += elapsed(
                suite, test, f"mark:{suite}.{test}-sync-end", f"mark:{suite}.{test}-async-end"
            )
            row["callbacks"] += elapsed(suite, test, "will-run-start", "will-run-end")
            row["callbacks"] += elapsed(suite, test, "run-test-callback", "did-run-end")
        rows[suite] = row

    totals = {column: sum(row[column] for row in rows.values()) for column in columns}
    benchmark_start = named_timestamps["benchmark-start"]
    benchmark_end = named_timestamps["finished"]
    wall_time = benchmark_end - benchmark_start
    accounted_time = sum(totals.values())
    runner_time = wall_time - accounted_time
    excluded_time = wall_time - totals["measured"]

    print("StyleBench phases (ms)")
    print(
        f"{'Suite':39} {'Prepare':>9} {'1st render':>10} {'Timers':>9} "
        f"{'Preflight':>10} {'Measured':>10} {'Post-render':>12} {'Callbacks':>10}"
    )
    for suite, row in rows.items():
        short_suite = suite.removesuffix(" selectors")
        print(
            f"{short_suite[:39]:39} {row['prepare']:9.1f} {row['first-render']:10.1f} "
            f"{row['later-timers']:9.1f} {row['preflight']:10.1f} {row['measured']:10.1f} "
            f"{row['post-render']:12.1f} {row['callbacks']:10.1f}"
        )
    print(
        f"{'Total':39} {totals['prepare']:9.1f} {totals['first-render']:10.1f} "
        f"{totals['later-timers']:9.1f} {totals['preflight']:10.1f} {totals['measured']:10.1f} "
        f"{totals['post-render']:12.1f} {totals['callbacks']:10.1f}"
    )

    print("\nOverall")
    print(f"Wall time:                         {wall_time:9.1f} ms")
    print(f"Measured sync test bodies:         {totals['measured']:9.1f} ms")
    print(f"Outside measured sync test bodies: {excluded_time:9.1f} ms")
    print(f"Runner/navigation/finalization:    {runner_time:9.1f} ms")

    print("\nSuite construction (ms, nested phases indented)")
    print(f"{'Suite':39} {'Time':>9}")

    def first_duration(suite, name):
        return durations[(suite, name)][0]

    for suite in suite_order:
        construct = first_duration(suite, "construct-benchmark")
        make_style = first_duration(suite, "make-style")
        generate_stylesheet = first_duration(suite, "generate-stylesheet")
        make_tree = first_duration(suite, "make-tree")
        cache_elements = first_duration(suite, "cache-elements")
        print(f"{suite.removesuffix(' selectors')[:39]:39} {construct:9.1f}")
        print(f"  Generate stylesheet text             {generate_stylesheet:9.1f}")
        print(f"  Parse and attach stylesheet           {make_style - generate_stylesheet:9.1f}")
        print(f"  Build and append element tree         {make_tree - cache_elements:9.1f}")
        print(f"  Cache initial test elements           {cache_elements:9.1f}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--webdriver", type=Path, default=DEFAULT_WEBDRIVER)
    parser.add_argument("--stylebench", type=Path, default=DEFAULT_STYLEBENCH)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--keep-stderr", action="store_true")
    parser.add_argument("--json", type=Path)
    arguments = parser.parse_args()

    profile = run_profile(arguments)
    if arguments.json:
        arguments.json.write_text(json.dumps(profile, indent=2) + "\n")
    print_profile(profile)


if __name__ == "__main__":
    main()
