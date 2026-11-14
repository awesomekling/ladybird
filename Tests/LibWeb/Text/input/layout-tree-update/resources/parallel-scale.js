// Helpers for tests whose documents are large enough for layout work to be split across threads: at least two
// sibling block subtrees with well over 128 boxes in total.

function repeatMarkup(count, makeMarkup) {
    let markup = "";
    for (let i = 0; i < count; ++i) markup += makeMarkup(i);
    return markup;
}

// Summarizes the layout subtree of a node as box counts per kind plus the text of all line box fragments in tree
// order, which shows generated text such as counter values and quotes.
function summarizeLayoutSubtree(node) {
    const kinds = new Map();
    const texts = [];
    let boxes = 0;
    for (const line of internals.dumpLayoutTree(node).split("\n")) {
        const trimmed = line.trim();
        if (trimmed.startsWith('"')) {
            texts.push(trimmed.slice(1, -1));
            continue;
        }
        const match = trimmed.match(/^(\w+) <([^>]*)>/);
        if (!match) continue;
        const kind = match[2] === "(anonymous)" ? `${match[1]}(anonymous)` : match[1];
        kinds.set(kind, (kinds.get(kind) ?? 0) + 1);
        ++boxes;
    }
    const kindList = [...kinds.entries()]
        .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
        .map(([kind, count]) => `${kind}=${count}`)
        .join(" ");
    return { boxes, kinds: kindList, text: texts.join(" ") };
}

function printLayoutSummaries(nodes) {
    let total = 0;
    for (const node of nodes) {
        const summary = summarizeLayoutSubtree(node);
        total += summary.boxes;
        println(`${node.id || node.nodeName}: ${summary.boxes} boxes: ${summary.kinds}`);
        if (summary.text) println(`  text: ${summary.text}`);
    }
    println(`total: ${total} boxes`);
}

function printFullRebuildComparison(name) {
    document.body.offsetHeight;
    const comparison = internals.compareLayoutTreeWithFullRebuild();
    println(`${comparison.matches ? "PASS" : "FAIL"}: ${name}`);
}

// Applies each mutation in turn to the same document and compares the incrementally updated tree with a full
// rebuild after each one.
function runSequentialFullRebuildComparisons(mutations) {
    printFullRebuildComparison("first build");
    for (const [name, mutate] of mutations) {
        mutate();
        printFullRebuildComparison(name);
    }
}
