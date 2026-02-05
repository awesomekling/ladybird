// Test that reassigning a function argument inside a loop works correctly.
// The loop condition must use the updated argument value, not the original.
function countdown(n) {
    var sum = 0;
    while (n > 0) {
        sum = sum + n;
        n = n - 1;
    }
    return sum;
}
countdown(5);
