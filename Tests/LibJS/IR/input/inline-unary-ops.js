function complement(x) {
    "use strict";
    return ~x >>> 0;
}
function caller() {
    return complement(0xff);
}
caller();
caller();
