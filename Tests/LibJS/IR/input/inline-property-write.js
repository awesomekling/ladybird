function setX(obj, val) {
    "use strict";
    obj.x = val;
    return obj;
}
var o = { x: 0 };
function caller() {
    return setX(o, 42);
}
caller();
caller();
