function abs(x) {
    "use strict";
    var result = x;
    if (x < 0) result = -x;
    return result;
}
function caller() {
    return abs(-5);
}
caller();
caller();
