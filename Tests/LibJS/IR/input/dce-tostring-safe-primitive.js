// Test that ToString on a safe primitive with unused result is eliminated by DCE.
function go(x) {
    var b = x === 1;
    `${b}`;
    return x;
}
go(1);
