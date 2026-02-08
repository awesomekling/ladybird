function add(a, b, c) {
    "use strict";
    return a + b + c;
}
function caller() {
    return add(1, 2, 3);
}
caller();
caller();
