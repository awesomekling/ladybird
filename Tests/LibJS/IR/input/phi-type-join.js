// Test that phi nodes joining int32 and number produce number, not unknown.
function go(arg) {
    let a = 5;
    let x = 6.5;
    let c;

    if (arg) {
        c = a;
    } else {
        c = x;
    }

    return c + 3;
}

go();
