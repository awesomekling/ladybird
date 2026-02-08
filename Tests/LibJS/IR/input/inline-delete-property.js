function removeProp(obj) {
    "use strict";
    delete obj.temp;
    return obj;
}
var o = { temp: 1, keep: 2 };
function caller() {
    return removeProp(o);
}
caller();
caller();
