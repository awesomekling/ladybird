// Test that LCSSA handles nested loops correctly by processing
// innermost loops first and inserting phis at each loop's exits.
function f(n) {
    var total = 0;
    for (var i = 0; i < n; i++) {
        for (var j = 0; j < n; j++) {
            total = total + j;
        }
        total = total + i;
    }
    return total;
}
f(5);
