// Test that the InlineCalls pass handles arithmetic helper functions.
function square(x) {
    return x * x;
}
function sum_of_squares(a, b) {
    return square(a) + square(b);
}
sum_of_squares(3, 4);
