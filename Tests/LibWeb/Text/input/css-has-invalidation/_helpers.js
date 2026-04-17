// Shared helpers for :has() invalidation tests.
// Each test includes include.js first, then this file.

function printCounters(label) {
    const c = internals.getStyleInvalidationCounters();
    println(`[${label}]`);
    println(`  hasAncestorWalkInvocations: ${c.hasAncestorWalkInvocations}`);
    println(`  hasAncestorWalkVisits: ${c.hasAncestorWalkVisits}`);
    println(`  hasMatchInvocations: ${c.hasMatchInvocations}`);
    println(`  hasResultCacheHits: ${c.hasResultCacheHits}`);
    println(`  hasResultCacheMisses: ${c.hasResultCacheMisses}`);
    println(`  styleInvalidations: ${c.styleInvalidations}`);
}

// Force a style pass and reset counters so a subsequent mutation can be
// measured in isolation.
function settleAndReset(triggerElement) {
    getComputedStyle(triggerElement || document.documentElement).color;
    internals.resetStyleInvalidationCounters();
}
