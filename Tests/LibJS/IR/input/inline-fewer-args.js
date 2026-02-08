function third(a, b, c) {
    "use strict";
    return c;
}
function caller() {
    return third(1, 2);
}
caller();
caller();
