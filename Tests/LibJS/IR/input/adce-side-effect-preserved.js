// Test ADCE: one branch arm has side effects (console.log), so the
// branch controlling it must be preserved even though the result
// is unused.
function go(x) {
    if (x > 0) {
        console.log("hello");
    }
    return 42;
}
go(1);
