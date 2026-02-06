// Test that exception handler edge copies are placed at the start of
// the block, not before the terminator. When any instruction throws,
// the handler phi must already have the correct value.
function ehEdge(f, g) {
    var x = "initial";
    try {
        x = f();
        x = g();
    } catch (e) {
        return x;
    }
    return x;
}
ehEdge(
    () => 1,
    () => 2
);
