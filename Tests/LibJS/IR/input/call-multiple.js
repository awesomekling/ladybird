// Test that multiple Call instructions each get distinct profile indices.
function a() {
    return 1;
}
function b() {
    return 2;
}
function caller() {
    return a() + b();
}
caller();
