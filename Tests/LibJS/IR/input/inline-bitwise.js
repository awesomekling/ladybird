function maskByte(a, b) {
    "use strict";
    return (a | b) & 0xff;
}
function caller() {
    return maskByte(0x0f, 0xf0);
}
caller();
caller();
