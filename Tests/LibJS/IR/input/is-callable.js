// Test that IsCallable is properly lifted as its own opcode.
// Array.fromAsync uses IsCallable internally (via JS spec implementation).
// Calling it exercises the IsCallable IR path during tier-up.
function go() {
    var result = Array.fromAsync([1, 2, 3], x => x * 2);
    return result;
}
go();
