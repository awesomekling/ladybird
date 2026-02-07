// Test that SimplifyCFG doesn't drop phi values when eliminating empty blocks
// whose predecessors are also implicit predecessors of the target (via finalizer).
// The catch handler modifies x, and the finally block must see the modified value.
function test(n) {
    let x = n | 0;
    try {
        if (((x & 1) | 0) === 0) throw (x ^ 95) | 0;
        x = (x + 95) | 0;
    } catch (e) {
        x = ((e | 0) + 17) | 0;
    } finally {
        x = (x + 15) | 0;
    }
    return x | 0;
}
test(0);
test(1);
test(2);
test(3);
