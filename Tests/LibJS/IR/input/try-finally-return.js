// Test return through try/finally (ScheduleJump, SetSavedReturnValue)
function foo() {
    try {
        return 42;
    } finally {
        var x = 1;
    }
}
foo();
