// Test labeled for loop with continue (exercises LoopSimplify phi handling).
function go() {
    var sum = 0;
    outer: for (var i = 0; i < 3; i++) {
        for (var j = 0; j < 3; j++) {
            if (j === 1) continue outer;
            sum = sum + 1;
        }
    }
    return sum;
}
go();
