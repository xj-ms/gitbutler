import {
	codeContentToTokens,
	ignoreWhitespaceInHunk,
	parseHunk,
	SectionType,
} from "$lib/utils/diffParsing";
import { describe, expect, test } from "vitest";

describe.concurrent("codeContentToTokens", () => {
	test("returns one token array per line", () => {
		const result = codeContentToTokens("line1\nline2\nline3", undefined);
		expect(result).toHaveLength(3);
	});

	test("sanitizes HTML entities in plain text fallback", () => {
		const result = codeContentToTokens('<script>alert("xss")</script>', undefined);
		const html = result[0].join("");
		expect(html).not.toContain("<script>");
		expect(html).toContain("&lt;script&gt;");
		expect(html).toContain("&quot;");
	});

	test("sanitizes ampersands", () => {
		const result = codeContentToTokens("a && b", undefined);
		const html = result[0].join("");
		expect(html).toContain("&amp;&amp;");
	});

	test("sanitizes single quotes", () => {
		const result = codeContentToTokens("it's", undefined);
		const html = result[0].join("");
		expect(html).toContain("&#39;");
	});

	test("wraps content in span with data-no-drag", () => {
		const result = codeContentToTokens("hello", undefined);
		const html = result[0].join("");
		expect(html).toContain("data-no-drag");
	});
});

function render(hunkStr: string) {
	const { hunk, whitespaceOnly } = ignoreWhitespaceInHunk(parseHunk(hunkStr));
	const prefix = {
		[SectionType.Context]: " ",
		[SectionType.AddedLines]: "+",
		[SectionType.RemovedLines]: "-",
	};
	const lines = hunk.contentSections.flatMap((s) =>
		s.lines.map(
			(l) =>
				`${prefix[s.sectionType]}${l.beforeLineNumber ?? "_"}:${l.afterLineNumber ?? "_"} ${l.content}`,
		),
	);
	return { lines, whitespaceOnly };
}

describe.concurrent("ignoreWhitespaceInHunk", () => {
	test("indentation-only change becomes context with new content", () => {
		const result = render(
			["@@ -1,3 +1,3 @@", " fn a() {", "-foo();", "+    foo();", " }"].join("\n"),
		);
		expect(result.whitespaceOnly).toBe(true);
		expect(result.lines).toEqual([" 1:1 fn a() {", " 2:2     foo();", " 3:3 }"]);
	});

	test("trailing whitespace is ignored", () => {
		const result = render(["@@ -1,1 +1,1 @@", "-a = 1;  ", "+a = 1;"].join("\n"));
		expect(result.whitespaceOnly).toBe(true);
		expect(result.lines).toEqual([" 1:1 a = 1;"]);
	});

	test("real changes are kept next to whitespace changes", () => {
		const result = render(
			["@@ -1,2 +1,2 @@", "-a = 1;", "-b = 2;", "+  a = 1;", "+b = 3;"].join("\n"),
		);
		expect(result.whitespaceOnly).toBe(false);
		expect(result.lines).toEqual([" 1:1   a = 1;", "-2:_ b = 2;", "+_:2 b = 3;"]);
	});

	test("matches whitespace-equivalent lines across separate sections", () => {
		// Re-indenting around an unchanged `}` splits the backend hunk into non-adjacent blocks.
		const result = render(
			["@@ -1,4 +1,4 @@", "-if (x) {", "+    if (x) {", "-y();", " }", "+    y();", " z();"].join(
				"\n",
			),
		);
		expect(result.whitespaceOnly).toBe(false);
		expect(result.lines.filter((l) => !l.startsWith(" "))).toHaveLength(2);
		expect(result.lines[0]).toBe(" 1:1     if (x) {");
	});

	test("unequal section sizes keep unmatched lines", () => {
		const result = render(["@@ -1,1 +1,3 @@", "-a", "+ a", "+b", "+c"].join("\n"));
		expect(result.lines).toEqual([" 1:1  a", "+_:2 b", "+_:3 c"]);
	});

	test("whitespace-only blank line changes become context", () => {
		const result = render(["@@ -1,3 +1,3 @@", " a", "-", "+   ", " b"].join("\n"));
		expect(result.whitespaceOnly).toBe(true);
		expect(result.lines).toEqual([" 1:1 a", " 2:2    ", " 3:3 b"]);
	});

	test("blank line additions and removals are dropped", () => {
		const result = render(["@@ -1,2 +1,3 @@", " a", "+", " b"].join("\n"));
		expect(result.whitespaceOnly).toBe(true);
		expect(result.lines).toEqual([" 1:1 a", " 2:3 b"]);
	});
});
