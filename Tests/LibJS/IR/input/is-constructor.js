// Test that IsConstructor is properly lifted as its own opcode.
// Array.fromAsync uses IsConstructor internally (via JS spec implementation).
// Calling it exercises the IsConstructor IR path during tier-up.
function go() {
    var result = Array.fromAsync({ length: 2, 0: "a", 1: "b" });
    return result;
}
go();
