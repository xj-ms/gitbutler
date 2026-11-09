import FileListItemContainer from "$components/files/FileListItemContainer.svelte";
import { DROPZONE_REGISTRY } from "$lib/dragging/registry";
import { FILE_SELECTION_MANAGER } from "$lib/selection/fileSelectionManager.svelte";
import { key } from "$lib/selection/key";
import { UNCOMMITTED_SERVICE } from "$lib/selection/uncommittedService.svelte";
import { STACK_SERVICE } from "$lib/stacks/stackService.svelte";
import { UI_STATE } from "$lib/state/uiState.svelte";
import { DRAG_STATE_SERVICE } from "@gitbutler/ui/drag/dragStateService.svelte";
import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, test, vi } from "vitest";
import type { SelectionId } from "$lib/selection/key";
import type { TreeChange } from "@gitbutler/but-sdk";

vi.mock("$components/shared/ChangedFilesContextMenu.svelte", () => ({
	default: () => ({}),
}));

vi.mock("$lib/dragging/draggable", () => ({
	draggableChips: () => ({}),
}));

function renderFile(
	path: string,
	selectionId: SelectionId,
	target?: HTMLElement,
	lockedCommitIds: string[] = [],
	mode: "list" | "tree" = "list",
) {
	const context = new Map<symbol, unknown>([
		[FILE_SELECTION_MANAGER._key, {}],
		[UNCOMMITTED_SERVICE._key, { fileCheckStatus: () => ({ current: "unchecked" }) }],
		[DROPZONE_REGISTRY._key, {}],
		[DRAG_STATE_SERVICE._key, {}],
		[STACK_SERVICE._key, { stacks: () => ({ result: { data: [] } }) }],
		[UI_STATE._key, { global: { pathFirst: { current: true } } }],
	]);
	const change: TreeChange = {
		path,
		pathBytes: [],
		status: {
			type: "Addition",
			subject: { state: { id: "blob", kind: "Blob" }, isUntracked: false },
		},
	};
	render(FileListItemContainer, {
		target,
		context,
		props: {
			projectId: "project",
			selectionId,
			change,
			listMode: mode,
			locked: lockedCommitIds.length > 0,
			lockedCommitIds,
		},
	});
	return document.getElementById(key({ ...selectionId, path }))!.parentElement!;
}

function renderCommit(commitId: string) {
	const wrapper = document.createElement("div");
	wrapper.className = "commit-row-wrapper";
	const row = document.createElement("div");
	row.dataset.commitId = commitId;
	wrapper.append(row);
	document.body.append(wrapper);
	return { wrapper, row };
}

describe("file dependency hover", () => {
	test.each(["list", "tree"] as const)(
		"highlights only the matching file in dependent commits in %s mode and clears on leave",
		async (mode) => {
			const first = renderCommit("first");
			const second = renderCommit("second");
			const unrelated = renderCommit("unrelated");
			// Quotes and brackets must be compared as a path, not interpreted as a CSS selector.
			const path = 'src/[special]"file.ts';
			const firstFile = renderFile(
				path,
				{ type: "commit", commitId: "first" },
				first.wrapper,
				[],
				mode,
			);
			const otherFile = renderFile(
				"other/file.ts",
				{ type: "commit", commitId: "first" },
				first.wrapper,
			);
			const secondFile = renderFile(
				path,
				{ type: "commit", commitId: "second" },
				second.wrapper,
				[],
				mode,
			);
			const unrelatedFile = renderFile(
				path,
				{ type: "commit", commitId: "unrelated" },
				unrelated.wrapper,
			);
			const uncommittedFile = renderFile(path, { type: "worktree" }, undefined, [
				"first",
				"second",
			]);
			const lock = screen.getByRole("img", { name: "File is locked due to dependencies" });

			await fireEvent.mouseEnter(lock);

			for (const element of [first.row, second.row, firstFile, secondFile]) {
				expect(element).toHaveClass("dependency-highlighted");
			}
			for (const element of [otherFile, unrelated.row, unrelatedFile, uncommittedFile]) {
				expect(element).not.toHaveClass("dependency-highlighted");
			}

			await fireEvent.mouseLeave(lock);

			expect(document.querySelectorAll(".dependency-highlighted")).toHaveLength(0);
		},
	);

	test("still highlights a collapsed commit without a file list", async () => {
		const { row } = renderCommit("collapsed");
		renderFile("file.ts", { type: "worktree" }, undefined, ["collapsed"]);
		const lock = screen.getByRole("img", { name: "File is locked due to dependencies" });

		await fireEvent.mouseEnter(lock);
		expect(row).toHaveClass("dependency-highlighted");

		await fireEvent.mouseLeave(lock);
		expect(row).not.toHaveClass("dependency-highlighted");
	});
});
