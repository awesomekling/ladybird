// Test that CopyCoalescing does not coalesce comparison results that
// feed a Branch, since the Lowerer may fuse them into a single
// JumpStrictlyEquals/JumpLessThan instruction that skips the register write.
function valueToString(value) {
    try {
        if (value === 0) {
            if (1 / value < 0) {
                return "-0";
            }
        }
        return String(value);
    } catch {
        return Object.prototype.toString.call(value);
    }
}
valueToString(42);
