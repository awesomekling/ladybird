// Test that !param is hoisted out of the loop when param is invariant.
function go(a) {
    var result;
    for (var i = 0; i < 10; i++) {
        result = !a;
    }
    return result;
}
go(true);
