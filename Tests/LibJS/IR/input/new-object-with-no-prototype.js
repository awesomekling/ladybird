// Test that NewObjectWithNoPrototype is properly lifted as its own opcode.
// GetIteratorFromMethod uses NewObjectWithNoPrototype to create iterator
// records. Array.fromAsync with a sync iterable exercises this path.
function go() {
    return Array.fromAsync([1, 2, 3]);
}
go();
