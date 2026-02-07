// Test that type facts from branch edges don't leak into merge blocks.
// After a branch on `x === undefined`, the merge point should still
// consider x as potentially undefined.
function go(x) {
    var first;
    if (x === undefined) {
        first = "yes";
    } else {
        first = "no";
    }
    // At the merge point, x is still unknown — this must NOT be folded.
    if (x === undefined) {
        return first + "-undefined";
    }
    return first + "-defined";
}
var result = go(undefined);
if (result !== "yes-undefined") throw new Error("Expected yes-undefined but got " + result);
result = go(42);
if (result !== "no-defined") throw new Error("Expected no-defined but got " + result);
