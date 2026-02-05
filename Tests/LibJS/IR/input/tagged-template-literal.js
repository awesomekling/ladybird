// Test that GetTemplateObject is properly lifted and lowered through IR.
function tag(strings) {
    return strings[0];
}
function go() {
    return tag`hello`;
}
go();
