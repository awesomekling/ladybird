// Test try/finally IR generation with exception/finalizer edges
var result = 0;
try {
    result = 1;
} finally {
    result = result + 10;
}
result;
