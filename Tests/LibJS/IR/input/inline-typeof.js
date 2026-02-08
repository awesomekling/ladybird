function isNumber(x) {
    "use strict";
    return typeof x === "number";
}
function caller() {
    return isNumber(42);
}
caller();
caller();
