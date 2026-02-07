// Test that phi nodes merging two different parameters do not incorrectly
// coalesce them together (parameters interfere with each other).
function choose(lhs, rhs) {
    return lhs && rhs;
}
choose(true, false);
