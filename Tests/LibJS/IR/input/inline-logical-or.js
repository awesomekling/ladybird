function either(a, b) {
    "use strict";
    return a || b;
}
function caller() {
    return either(0, 42);
}
caller();
caller();
