function sideEffect(x) {
    "use strict";
    x.count = (x.count || 0) + 1;
}
var obj = {};
function caller() {
    "use strict";
    sideEffect(obj);
    return obj.count;
}
caller();
caller();
