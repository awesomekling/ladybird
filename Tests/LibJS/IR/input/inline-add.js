function add(a, b) {
    "use strict";
    return a + b;
}
function caller() {
    return add(3, 4);
}
caller();
caller();
