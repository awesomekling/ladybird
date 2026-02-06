// Test that functions with 'var arguments' work correctly after IR
// optimization. The CreateArguments instruction must preserve its dst
// even when the arguments object value becomes unused through optimization.
function f1() {
    var arguments = 42;
    return arguments;
}

function f2() {
    var arguments = [1, 2];
    return arguments.length;
}

if (f1() !== 42) throw "f1 failed";
if (f2() !== 2) throw "f2 failed";
