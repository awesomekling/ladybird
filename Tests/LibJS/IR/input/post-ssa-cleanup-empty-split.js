// Test PostSSACleanup: empty split blocks (Jump only) get eliminated.
// A do-while with a conditional body creates critical edges that
// SplitCriticalEdges splits into empty blocks.
function loopWithBranch(n) {
    var x = 0;
    do {
        if (n > 5) {
            x = x + 1;
        }
        n = n - 1;
    } while (n > 0);
    return x;
}
loopWithBranch(10);
