// Test that CreateAsyncFromSyncIterator preserves all three operands.
// Array.fromAsync with a sync iterable triggers CreateAsyncFromSyncIterator
// in the internal JS implementation.
function go() {
    return Array.fromAsync([1, 2, 3]);
}
go();
