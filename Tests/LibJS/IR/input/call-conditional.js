// Test Call in conditional branches round-trips through IR.
function yes() {
    return true;
}
function no() {
    return false;
}
function caller(x) {
    if (x > 0) return yes();
    else return no();
}
caller(1);
