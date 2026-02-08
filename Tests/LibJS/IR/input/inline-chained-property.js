function deepGet(obj) {
    "use strict";
    return obj.a.b;
}
var deep = { a: { b: 99 } };
function caller() {
    return deepGet(deep);
}
caller();
caller();
