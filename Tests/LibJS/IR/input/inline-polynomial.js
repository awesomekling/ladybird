function poly(x) {
    "use strict";
    return x * x * x + 2 * x * x + 3 * x + 4;
}
function caller() {
    return poly(5);
}
caller();
caller();
