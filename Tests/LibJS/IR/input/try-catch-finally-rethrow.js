// Test that exceptions thrown in catch blocks are re-thrown after finally
var result = "";
try {
    try {
        throw "first";
    } catch (e) {
        result = result + "catch,";
        throw "second";
    } finally {
        result = result + "finally,";
    }
} catch (e) {
    result = result + e;
}
result;
