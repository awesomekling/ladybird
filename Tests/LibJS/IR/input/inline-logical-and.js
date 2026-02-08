function both(a, b) {
    "use strict";
    return a && b;
}
function caller() {
    return both(1, 2);
}
caller();
caller();
