// Test that ExtractValue with different indices from the same tuple are NOT merged.
// GetIterator produces a tuple; ExtractValue at index 0 vs 1 must differ.
function go(arr) {
    for (var x of arr) {
        return x;
    }
}
go([42]);
