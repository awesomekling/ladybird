function getX(obj) {
    "use strict";
    return obj.x;
}
var point = { x: 42, y: 10 };
function caller() {
    return getX(point);
}
caller();
caller();
