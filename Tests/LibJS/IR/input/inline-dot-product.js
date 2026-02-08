function dot(a, b) {
    "use strict";
    return a.x * b.x + a.y * b.y;
}
var v1 = { x: 3, y: 4 };
var v2 = { x: 5, y: 6 };
function caller() {
    return dot(v1, v2);
}
caller();
caller();
