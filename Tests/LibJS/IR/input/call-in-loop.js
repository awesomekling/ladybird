// Test Call inside a loop round-trips through IR correctly.
function inc(x) {
    return x + 1;
}
function caller(n) {
    var result = 0;
    for (var i = 0; i < n; i++) {
        result = inc(result);
    }
    return result;
}
caller(5);
