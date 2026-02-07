// Test ADCE: generator function with dead branches. Yield terminators
// must be treated as exit blocks in the post-dominator tree so that
// dead branch conditions are not incorrectly removed.
function* gen(x) {
    var y;
    if (x) {
        y = 1;
    } else {
        y = 2;
    }
    yield 42;
}
var it = gen(true);
it.next();
