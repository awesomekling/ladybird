// Test type refinement: known-type values have IsNullish folded.
// NewObject produces a value with type object, so IsNullish should
// fold to false. (The for-in pattern naturally produces this.)
function go() {
    var obj = {};
    if (obj == null) {
        return "nullish";
    }
    return "not nullish";
}
go();
