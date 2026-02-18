"use strict";
const x = String.raw`hello`;
let y = null;
if (!isInBrowser) {
    y = makeBenchmarkRunner("arg");
}
