// Deterministic holds on the frame in flight (LIBWEB_STAGE_THREAD=overlap).
//
// whileFrameInFlight(point, mutate, during) runs `mutate` in a rAF callback, so the rendering update paints, and holds
// the recording that update submits at `point` ("before-run", "mid-recording" or "before-completion"). `during`
// then runs in a task while that frame is held there, and gets { armed, heldAt, state }. With `doc`, only that
// document's recording is held (with iframes, hold the one the rendering update paints last: a main-thread wait for
// the render side, such as the layout update of a document painted after it, lets a held recording go on). Where frames are not
// submitted (default and lockstep modes), nothing is armed and `during` runs after the rendering update: a test prints
// the same output in every mode, and checks the in-flight facts only when `armed` is set.
// It starts once the document has loaded: the load task lays the document out, which waits for the frame in flight.
// Where the rendering update submits its layout pass too (LIBWEB_STAGE_OVERLAP naming "layout"), the frame is in
// flight twice: the recording is submitted only once the main thread has taken the layout pass back between tasks and
// gone on with the rendering update, so `during` waits for that first.
async function whileFrameInFlight(point, mutate, during, doc = null) {
    if (document.readyState !== "complete")
        await new Promise(resolve => window.addEventListener("load", resolve, { once: true }));
    return new Promise((resolve, reject) => {
        requestAnimationFrame(() => {
            const armed = internals.holdNextRecordingFrame(point, doc);
            mutate();
            setTimeout(async () => {
                try {
                    while (armed && internals.renderingUpdateAwaitsPass()) {
                        internals.waitForFrameToFinish();
                        await nextTask();
                    }
                    const heldAt = armed ? internals.waitForHeldFrame() : "";
                    const frame = { armed, heldAt, state: internals.frameSchedulerState() };
                    const result = await during(frame);
                    internals.releaseHeldFrame();
                    resolve(result);
                } catch (e) {
                    internals.releaseHeldFrame();
                    reject(e);
                }
            }, 0);
        });
    });
}

// whileLayoutInFlight(point, mutate, during) is whileFrameInFlight for the full layout pass a rendering update submits
// under LIBWEB_STAGE_OVERLAP=layout, held at `point` ("before-run" or "before-completion"): `mutate` runs in a rAF
// callback and has to leave layout to do, and `during` runs in a task while that pass is held. With `doc`, only that
// document's pass is held. `during` gets { heldAt, state }: heldAt is "" wherever no layout pass was submitted (every
// mode that does not submit one, and a rendering update that lays out in place), and `during` then runs after the
// rendering update. A test prints the same output in every mode, and checks the in-flight facts only when heldAt is set.
async function whileLayoutInFlight(point, mutate, during, doc = null) {
    if (document.readyState !== "complete")
        await new Promise(resolve => window.addEventListener("load", resolve, { once: true }));
    return new Promise((resolve, reject) => {
        requestAnimationFrame(() => {
            const armed = internals.holdNextLayoutFrame(point, doc);
            mutate();
            setTimeout(async () => {
                try {
                    // Returns "" at once if no layout pass was submitted.
                    const heldAt = armed ? internals.waitForHeldFrame() : "";
                    const frame = { heldAt, state: internals.frameSchedulerState() };
                    const result = await during(frame);
                    internals.releaseHeldFrame();
                    resolve(result);
                } catch (e) {
                    internals.releaseHeldFrame();
                    reject(e);
                }
            }, 0);
        });
    });
}

// whileStyleInFlight(point, mutate, during) is whileLayoutInFlight for the first style pass a rendering update submits
// under LIBWEB_STAGE_OVERLAP naming "style", held at `point` ("before-run" or "before-completion"): `mutate` runs in a
// rAF callback and has to leave style to do, and `during` runs in a task while that pass is held. With `doc`, only that
// document's pass is held. `during` gets { heldAt, state }: heldAt is "" wherever no style pass was submitted, and
// `during` then runs after the rendering update. A test prints the same output in every mode, and checks the in-flight
// facts only when heldAt is set (styleHeldAsArmed).
async function whileStyleInFlight(point, mutate, during, doc = null) {
    if (document.readyState !== "complete")
        await new Promise(resolve => window.addEventListener("load", resolve, { once: true }));
    return new Promise((resolve, reject) => {
        requestAnimationFrame(() => {
            const armed = internals.holdNextStyleFrame(point, doc);
            mutate();
            setTimeout(async () => {
                try {
                    // Returns "" at once if no style pass was submitted.
                    const heldAt = armed ? internals.waitForHeldFrame() : "";
                    const frame = { heldAt, state: internals.frameSchedulerState() };
                    const result = await during(frame);
                    internals.releaseHeldFrame();
                    resolve(result);
                } catch (e) {
                    internals.releaseHeldFrame();
                    reject(e);
                }
            }, 0);
        });
    });
}

// Whether running `write` took a style pass in flight back (a forced join) instead of leaving its style input to wait
// for the pass's drain. False wherever no style pass is in flight.
function writeJoinedStylePass(write) {
    const before = internals.stylePassForcedJoins();
    write();
    return internals.stylePassForcedJoins() !== before;
}

// Whether the style pass was held where it was armed and was in flight while it was (true wherever none was held).
function styleHeldAsArmed(frame, point) {
    return !frame.heldAt || (frame.heldAt === point && frame.state === "in-flight");
}

// Whether the layout pass was held where it was armed and was in flight while it was (true wherever none was held).
function layoutHeldAsArmed(frame, point) {
    return !frame.heldAt || (frame.heldAt === point && frame.state === "in-flight");
}

// Whether the frame was held where it was armed and was in flight while it was (true in every mode that did not arm).
function heldAsArmed(frame, point) {
    return !frame.armed || (frame.heldAt === point && frame.state === "in-flight");
}

function nextTask() {
    const { promise, resolve } = Promise.withResolvers();
    const channel = new MessageChannel();
    channel.port1.onmessage = resolve;
    channel.port2.postMessage(null);
    return promise;
}

function nextFrame() {
    return new Promise(resolve => requestAnimationFrame(() => resolve()));
}

async function twoFrames() {
    await nextFrame();
    await nextFrame();
}
