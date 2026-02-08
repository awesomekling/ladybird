function clamp(x) {
    "use strict";
    var result = x;
    if (x > 100) result = 100;
    return result;
}
function caller() {
    return clamp(150);
}
caller();
caller();
