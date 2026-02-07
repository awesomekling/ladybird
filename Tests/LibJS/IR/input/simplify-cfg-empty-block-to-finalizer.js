// Test that SimplifyCFG doesn't eliminate an empty block when doing so would
// create a terminator edge to the block's own finalizer with phi nodes.
function f(x) {
    var mut = 0;
    for (var i = 0; i < 2; i++) {
        try {
            for (var j = 0; j < 3; j++) {
                var v = x + i + j;
                if ((v & 3) === 1) continue;
                mut = mut + v - j;
            }
        } finally {
            mut = (mut + i + 1) * 2;
        }
    }
    return mut;
}
var result = f(5);
