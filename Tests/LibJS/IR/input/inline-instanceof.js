function isArray(x) {
    "use strict";
    return x instanceof Array;
}
function caller() {
    return isArray([1, 2, 3]);
}
caller();
caller();
