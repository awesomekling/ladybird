// Test ADCE: a do-while loop where the iteration variable is unused
// after the loop. ADCE should eliminate the loop's back-edge branch
// since the condition only feeds dead control flow.
function go(x) {
    do {} while (x);
    return 42;
}
go(false);
