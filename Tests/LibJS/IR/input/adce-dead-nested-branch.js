// Test ADCE: nested do-while loops where the outer loop variable is
// unused. Both loop back-edge branches should be eliminated.
function go(x, y) {
    do {
        do {} while (y);
    } while (x);
    return 0;
}
go(false, false);
