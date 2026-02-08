function square(x) {
    "use strict";
    return x * x;
}
function caller(a, b) {
    "use strict";
    return square(a) + square(b);
}
caller(3, 4);
caller(3, 4);
