import {
	amendTarget,
	amendWorktreeChanges,
	runAmendWorktreeChanges,
	stageTarget,
} from "$lib/commits/stageOrAmend.svelte";
import { hunkToDiffSpec } from "$lib/commits/utils";
import { HunkDropDataV3 } from "$lib/dragging/draggables";
import { AssignmentDropHandler } from "$lib/dragging/dropHandlers/hunkDropHandler";
import { HookFailedError, type HooksService } from "$lib/git/hooksService";
import { describe, expect, test, vi } from "vitest";
import type { DiffService } from "$lib/hunks/diffService.svelte";
import type { FileSelectionManager } from "$lib/selection/fileSelectionManager.svelte";
import type { SelectionId } from "$lib/selection/key";
import type { UncommittedService } from "$lib/selection/uncommittedService.svelte";
import type { StackService } from "$lib/stacks/stackService.svelte";
import type { UiState } from "$lib/state/uiState.svelte";
import type { DiffHunk, DiffSpec, TreeChange } from "@gitbutler/but-sdk";

vi.mock("$lib/error/showError", () => ({
	showError: vi.fn(),
}));

const sampleChanges: DiffSpec[] = [
	{
		previousPathBytes: null,
		pathBytes: [97, 46, 116, 115],
		hunkHeaders: [],
	},
];

function makeMockUiState(selectedCommitId?: string) {
	const laneSelection = {
		current: selectedCommitId ? { branchName: "feature/a", commitId: selectedCommitId } : undefined,
		set: vi.fn((next) => {
			laneSelection.current = next;
		}),
	};
	const modalSet = vi.fn();
	const stackBusySet = vi.fn();

	const uiState = {
		project: vi.fn(() => ({
			stackBusy: { set: stackBusySet },
		})),
		lane: vi.fn(() => ({
			selection: laneSelection,
		})),
		global: {
			modal: { set: modalSet },
		},
	} as unknown as UiState;

	return { uiState, laneSelection, modalSet, stackBusySet };
}

describe("stageOrAmend", () => {
	test("amendWorktreeChanges runs hooks, updates selected commit, and surfaces rejected changes", async () => {
		const { uiState, laneSelection, modalSet } = makeMockUiState("old-commit");
		const runPreCommitHooks = vi.fn().mockResolvedValue(undefined);
		const runPostCommitHooks = vi.fn().mockResolvedValue(undefined);
		const amendCommitMutation = vi.fn().mockResolvedValue({
			newCommit: "new-commit",
			rejectedChanges: [{ path: "b.ts", reason: "cherryPickMergeConflict" }],
		});

		const outcome = await amendWorktreeChanges({
			projectId: "proj-1",
			stackId: "stack-1",
			commitId: "old-commit",
			targetBranchName: "feature/a",
			worktreeChanges: sampleChanges,
			runHooks: true,
			hooksService: {
				runPreCommitHooks,
				runPostCommitHooks,
			} as unknown as HooksService,
			stackService: {
				amendCommitMutation,
			} as unknown as StackService,
			uiState,
		});

		expect(runPreCommitHooks).toHaveBeenCalledExactlyOnceWith("proj-1", sampleChanges);
		expect(amendCommitMutation).toHaveBeenCalledExactlyOnceWith({
			projectId: "proj-1",
			stackId: "stack-1",
			worktree: undefined,
			commitId: "old-commit",
			worktreeChanges: sampleChanges,
			dryRun: false,
		});
		expect(laneSelection.set).toHaveBeenCalledExactlyOnceWith({
			branchName: "feature/a",
			commitId: "new-commit",
		});
		expect(modalSet).toHaveBeenCalledExactlyOnceWith({
			type: "commit-failed",
			projectId: "proj-1",
			targetBranchName: "feature/a",
			newCommitId: "new-commit",
			commitTitle: undefined,
			pathsToRejectedChanges: { "b.ts": "cherryPickMergeConflict" },
		});
		expect(runPostCommitHooks).toHaveBeenCalledExactlyOnceWith("proj-1");
		expect(outcome?.newCommit).toBe("new-commit");
	});

	test("amendWorktreeChanges aborts without amending on HookFailedError", async () => {
		const { uiState } = makeMockUiState();
		const runPreCommitHooks = vi.fn().mockRejectedValue(new HookFailedError());
		const amendCommitMutation = vi.fn();

		const outcome = await amendWorktreeChanges({
			projectId: "proj-1",
			stackId: "stack-1",
			commitId: "old-commit",
			worktreeChanges: sampleChanges,
			runHooks: true,
			hooksService: {
				runPreCommitHooks,
				runPostCommitHooks: vi.fn(),
			} as unknown as HooksService,
			stackService: {
				amendCommitMutation,
			} as unknown as StackService,
			uiState,
		});

		expect(outcome).toBeUndefined();
		expect(amendCommitMutation).not.toHaveBeenCalled();
	});

	test("stageTarget stages a single hunk and clears file selection when moving the last hunk", async () => {
		const hunk: DiffHunk = {
			oldStart: 10,
			oldLines: 3,
			newStart: 10,
			newLines: 4,
			diff: "@@ -10,3 +10,4 @@",
		};
		const change = {
			path: "file.ts",
			pathBytes: [102, 105, 108, 101],
			status: { type: "Modification" },
		} as unknown as TreeChange;
		const selectionId: SelectionId = { type: "worktree", stackId: undefined };
		const assignHunk = vi.fn().mockResolvedValue([]);
		const remove = vi.fn();
		const clearPreview = vi.fn();
		const unselectHunkLines = vi.fn();

		await stageTarget({
			projectId: "proj-1",
			sourceStackId: undefined,
			selectionId,
			target: { type: "hunk", change, hunk, unselectHunkLines },
			targetStackId: "stack-2",
			deps: {
				diffService: { assignHunk } as unknown as DiffService,
				idSelection: { remove, clearPreview } as unknown as FileSelectionManager,
				uncommittedService: {
					getAssignmentsByPath: () => [
						{
							hunkHeader: { oldStart: 10, oldLines: 3, newStart: 10, newLines: 4 },
							pathBytes: change.pathBytes,
						},
					],
				} as unknown as UncommittedService,
			},
		});

		expect(unselectHunkLines).toHaveBeenCalledExactlyOnceWith(hunk);
		expect(assignHunk).toHaveBeenCalledExactlyOnceWith({
			projectId: "proj-1",
			assignments: [
				{
					hunkHeader: { oldStart: 10, oldLines: 3, newStart: 10, newLines: 4 },
					pathBytes: change.pathBytes,
					target: { type: "stack", subject: { stackId: "stack-2" } },
				},
			],
		});
		expect(remove).toHaveBeenCalledExactlyOnceWith("file.ts", selectionId);
		expect(clearPreview).toHaveBeenCalledExactlyOnceWith(selectionId);
	});

	test("amendTarget keeps rejected files selected while deselecting amended files", async () => {
		const { uiState } = makeMockUiState();
		const removeMany = vi.fn();
		const clearPreview = vi.fn();
		const selectionId: SelectionId = { type: "worktree", stackId: "stack-1" };
		const changes = [
			{ path: "ok.ts", pathBytes: [1], status: { type: "Modification" } },
			{ path: "rejected.ts", pathBytes: [2], status: { type: "Modification" } },
		] as unknown as TreeChange[];

		await amendTarget({
			projectId: "proj-1",
			sourceStackId: "stack-1",
			selectionId,
			target: { type: "files", changes },
			targetStackId: "stack-1",
			commitId: "commit-1",
			targetBranchName: "feature/a",
			runHooks: false,
			deps: {
				hooksService: {} as unknown as HooksService,
				idSelection: { removeMany, clearPreview } as unknown as FileSelectionManager,
				stackService: {
					amendCommitMutation: vi.fn().mockResolvedValue({
						newCommit: "commit-2",
						rejectedChanges: [{ path: "rejected.ts", reason: "cherryPickMergeConflict" }],
					}),
				} as unknown as StackService,
				uiState,
				uncommittedService: {
					getAssignmentsByPaths: () => ({
						"ok.ts": [{ hunkHeader: null, pathBytes: [1] }],
						"rejected.ts": [{ hunkHeader: null, pathBytes: [2] }],
					}),
				} as unknown as UncommittedService,
			},
		});

		expect(removeMany).toHaveBeenCalledExactlyOnceWith([{ ...selectionId, path: "ok.ts" }]);
		expect(clearPreview).toHaveBeenCalledExactlyOnceWith(selectionId);
	});

	test("runAmendWorktreeChanges returns dropResult and invokes onNewCommit for drop handlers", async () => {
		const { uiState, laneSelection } = makeMockUiState("old-commit");
		const onNewCommit = vi.fn();

		const { dropResult } = await runAmendWorktreeChanges({
			projectId: "proj-1",
			stackId: "stack-1",
			commitId: "old-commit",
			worktreeChanges: sampleChanges,
			runHooks: false,
			hooksService: {} as unknown as HooksService,
			stackService: {
				amendCommitMutation: vi.fn().mockResolvedValue({
					newCommit: "new-commit",
					rejectedChanges: [{ path: "b.ts", reason: "cherryPickMergeConflict" }],
				}),
			} as unknown as StackService,
			uiState,
			onNewCommit,
		});

		expect(onNewCommit).toHaveBeenCalledExactlyOnceWith("new-commit");
		expect(laneSelection.set).not.toHaveBeenCalled();
		expect(dropResult).toEqual({
			type: "rejectedChanges",
			projectId: "proj-1",
			newCommitId: "new-commit",
			commitTitle: undefined,
			targetBranchName: "",
			pathsToRejectedChanges: { "b.ts": "cherryPickMergeConflict" },
		});
	});

	test("AssignmentDropHandler delegates hunk drops to shared stageTarget including whole-file additions", async () => {
		const change = {
			path: "added.ts",
			pathBytes: [97, 100, 100],
			status: { type: "Addition" },
		} as unknown as TreeChange;
		const selectionId: SelectionId = { type: "worktree", stackId: undefined };
		const assignHunk = vi.fn().mockResolvedValue([]);
		const remove = vi.fn();
		const clearPreview = vi.fn();

		const handler = new AssignmentDropHandler(
			"proj-1",
			{ assignHunk } as unknown as DiffService,
			{
				getAssignmentsByPath: () => [{ hunkHeader: null, pathBytes: change.pathBytes }],
			} as unknown as UncommittedService,
			"stack-1",
			{ remove, clearPreview } as unknown as FileSelectionManager,
		);

		await handler.ondrop(
			new HunkDropDataV3(
				change,
				{ oldStart: 0, oldLines: 0, newStart: 1, newLines: 5 },
				true,
				null,
				undefined,
				selectionId,
			),
		);

		expect(assignHunk).toHaveBeenCalledExactlyOnceWith({
			projectId: "proj-1",
			assignments: [
				{
					hunkHeader: null,
					pathBytes: change.pathBytes,
					target: { type: "stack", subject: { stackId: "stack-1" } },
				},
			],
		});
		expect(remove).toHaveBeenCalledExactlyOnceWith("added.ts", selectionId);
		expect(clearPreview).toHaveBeenCalledExactlyOnceWith(selectionId);
	});

	test("hunkToDiffSpec omits hunkHeaders for whole-file additions and preserves rename previousPathBytes", () => {
		const added = {
			path: "new.ts",
			pathBytes: [1, 2],
			status: { type: "Addition" },
		} as unknown as TreeChange;
		const renamed = {
			path: "renamed.ts",
			pathBytes: [3, 4],
			status: { type: "Rename", subject: { previousPathBytes: [9, 9] } },
		} as unknown as TreeChange;
		const header = { oldStart: 1, oldLines: 2, newStart: 1, newLines: 3 };

		expect(hunkToDiffSpec(added, header)).toEqual({
			previousPathBytes: null,
			pathBytes: [1, 2],
			hunkHeaders: [],
		});
		expect(hunkToDiffSpec(renamed, header)).toEqual({
			previousPathBytes: [9, 9],
			pathBytes: [3, 4],
			hunkHeaders: [header],
		});
	});

	test("amendTarget amends only selected lines and preserves file selection when unselected lines remain", async () => {
		const { uiState } = makeMockUiState();
		const hunk: DiffHunk = {
			oldStart: 1,
			oldLines: 2,
			newStart: 1,
			newLines: 4,
			diff: "@@ -1,2 +1,4 @@\n context\n+first added\n+second added\n trailing\n",
		};
		const change = {
			path: "partial.ts",
			pathBytes: [112, 97, 114],
			status: { type: "Modification" },
		} as unknown as TreeChange;
		const selectionId: SelectionId = { type: "worktree", stackId: undefined };
		const amendCommitMutation = vi.fn().mockResolvedValue({
			newCommit: "commit-2",
			rejectedChanges: [],
		});
		const remove = vi.fn();
		const clearPreview = vi.fn();
		const unselectHunkLines = vi.fn();

		await amendTarget({
			projectId: "proj-1",
			sourceStackId: undefined,
			selectionId,
			target: {
				type: "lines",
				change,
				hunk,
				selectedLines: [{ oldLine: undefined, newLine: 2 }],
				unselectHunkLines,
			},
			targetStackId: "stack-1",
			commitId: "commit-1",
			targetBranchName: "feature/a",
			runHooks: false,
			deps: {
				hooksService: {} as unknown as HooksService,
				idSelection: { remove, clearPreview } as unknown as FileSelectionManager,
				stackService: { amendCommitMutation } as unknown as StackService,
				uiState,
				uncommittedService: {
					getAssignmentsByPath: () => [
						{
							hunkHeader: { oldStart: 1, oldLines: 2, newStart: 1, newLines: 4 },
							pathBytes: change.pathBytes,
						},
					],
				} as unknown as UncommittedService,
			},
		});

		expect(unselectHunkLines).toHaveBeenCalledExactlyOnceWith(hunk);
		expect(amendCommitMutation).toHaveBeenCalledExactlyOnceWith({
			projectId: "proj-1",
			stackId: "stack-1",
			worktree: undefined,
			commitId: "commit-1",
			worktreeChanges: [
				{
					previousPathBytes: null,
					pathBytes: change.pathBytes,
					hunkHeaders: [{ oldStart: 0, oldLines: 0, newStart: 2, newLines: 1 }],
				},
			],
			dryRun: false,
		});
		expect(remove).not.toHaveBeenCalled();
		expect(clearPreview).not.toHaveBeenCalled();
	});
});
