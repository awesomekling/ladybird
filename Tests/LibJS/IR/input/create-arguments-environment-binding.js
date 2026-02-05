// Test that CreateArguments without a dst register properly creates the
// 'arguments' binding in the environment. This is important because
// CreateArguments with a dst register skips binding creation.
function foo() {
    return arguments.length;
}
foo(1, 2, 3);
