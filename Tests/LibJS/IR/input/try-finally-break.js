// Test break through try/finally (ScheduleJump)
function foo() {
    var result = 0;
    for (var i = 0; i < 3; i++) {
        try {
            if (i === 1) break;
        } finally {
            result = result + 1;
        }
    }
    return result;
}
foo();
