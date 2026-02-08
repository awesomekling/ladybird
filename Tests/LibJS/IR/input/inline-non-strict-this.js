function getX() {
    return this.x;
}
var obj = { x: 42, getX: getX };
function caller() {
    "use strict";
    return obj.getX();
}
caller();
caller();
