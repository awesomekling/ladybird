// Test that unused allocations (NewObject, NewArray, NewFunction, NewRegExp)
// are eliminated by DCE, and that they don't split basic blocks.
function go() {
    let o = {};
    let a = [1, 2, 3];
    return 3;
}
go();
