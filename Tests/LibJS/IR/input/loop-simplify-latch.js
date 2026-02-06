// Test that LoopSimplify merges multiple back-edges into a single latch.
// The conditional continue creates two back-edges to the loop header.
function go(a) {
    var sum = 0;
    for (var i = 0; i < 10; i++) {
        if (a) continue;
        sum = sum + i;
    }
    return sum;
}
go(false);
