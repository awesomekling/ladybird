function add(a, b) {
    "use strict";
    return a + b;
}
function mul(a, b) {
    "use strict";
    return a * b;
}
function caller() {
    return add(mul(2, 3), mul(4, 5));
}
caller();
caller();
