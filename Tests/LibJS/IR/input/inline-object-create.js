function makePoint(x, y) {
    "use strict";
    return { x: x, y: y };
}
function caller() {
    return makePoint(3, 4);
}
caller();
caller();
