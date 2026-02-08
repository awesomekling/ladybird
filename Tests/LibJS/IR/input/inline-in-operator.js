function hasKey(obj, key) {
    "use strict";
    return key in obj;
}
var obj = { x: 1, y: 2 };
function caller() {
    return hasKey(obj, "x");
}
caller();
caller();
