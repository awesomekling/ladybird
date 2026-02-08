function double(x) {
    "use strict";
    return x * 2;
}
function addOne(x) {
    "use strict";
    return x + 1;
}
function caller(x) {
    "use strict";
    return double(addOne(x));
}
caller(5);
caller(5);
