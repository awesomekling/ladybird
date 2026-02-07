// Test ADCE: a loop where the exit condition feeds a dead branch
// after the loop. The loop body has side effects so the loop itself
// must stay, but the post-loop branch should be removed if dead.
function go(x) {
    var i = 0;
    while (i < x) {
        console.log(i);
        i = i + 1;
    }
    var dead = i + 100;
    if (dead > 200) {
        var unused = dead * 2;
    }
    return i;
}
go(3);
