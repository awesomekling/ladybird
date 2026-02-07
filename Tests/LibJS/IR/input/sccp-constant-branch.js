// Test that SCCP eliminates dead branches when the condition is a constant.
// The false branch should be removed since `true` is always truthy.
function go() {
    var x = true;
    if (x) {
        return 42;
    } else {
        return 99;
    }
}
go();
