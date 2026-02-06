// Test basic SSA destruction: phi nodes are replaced with ParallelCopy
// instructions on normal edges (before the terminator).
function diamond(x) {
    var result;
    if (x > 0) {
        result = x + 1;
    } else {
        result = x - 1;
    }
    return result;
}
diamond(5);
