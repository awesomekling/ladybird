// Test that CopyCoalescing does not clear pre-existing interference
// when applying the phi exception for parallel copies.
//
// In a generator with a postfix-decrement loop like `for (var i = 5; i--; )`,
// when the phi has type `number`, InstCombine folds the ToNumeric so
// the Decrement operates directly on the phi result. The pre-decrement
// value (used by the branch) and the post-decrement value (used by the
// loop body and fed back to the phi) must NOT be coalesced into the
// same register, even though a phi copy connects them.
function* countdown(arr) {
    for (var i = 5; i--; ) yield arr[i];
}
var result = [...countdown(["a", "b", "c", "d", "e"])];
