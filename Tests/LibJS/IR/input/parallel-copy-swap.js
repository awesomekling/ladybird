// Test that ParallelCopy correctly resolves swap cycles.
// The loop back-edge has two copies that form a cycle:
// a_new <- b_old and b_new <- a_old. Without parallel resolution,
// one copy would clobber the source of the other.
function swap(n) {
    var a = 1;
    var b = 2;
    for (var i = 0; i < n; i++) {
        var tmp = a;
        a = b;
        b = tmp;
    }
    return a * 10 + b;
}
swap(1);
