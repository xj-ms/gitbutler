import StageOrAmendSubmenu from "$components/shared/StageOrAmendSubmenu.svelte";
import { HOOKS_SERVICE } from "$lib/git/hooksService";
import { DIFF_SERVICE } from "$lib/hunks/diffService.svelte";
import { FILE_SELECTION_MANAGER } from "$lib/selection/fileSelectionManager.svelte";
import { UNCOMMITTED_SERVICE } from "$lib/selection/uncommittedService.svelte";
import { STACK_SERVICE } from "$lib/stacks/stackService.svelte";
import { UI_STATE } from "$lib/state/uiState.svelte";
import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterAll, beforeAll, describe, expect, test, vi } from "vitest";
import type { StageOrAmendTarget } from "$lib/commits/stageOrAmend.svelte";
import type { SelectionId } from "$lib/selection/key";
import type { Stack } from "$lib/stacks/stack";
import type { TreeChange } from "@gitbutler/but-sdk";

function makeCommit(
	id: string,
	message: string,
	opts?: { hasConflicts?: boolean; integrated?: boolean },
) {
	return {
		id,
		message,
		hasConflicts: opts?.hasConflicts ?? false,
		state: opts?.integrated ? { type: "Integrated" } : { type: "LocalOnly" },
	};
}

function makeStack(
	id: string | null,
	segments: Array<{
		branchName: string;
		commits: ReturnType<typeof makeCommit>[];
	}>,
): Stack {
	return {
		id,
		segments: segments.map((seg) => ({
			refName: {
				displayName: seg.branchName,
				fullNameBytes: Array.from(new TextEncoder().encode(`refs/heads/${seg.branchName}`)),
			},
			commits: seg.commits,
		})),
	} as unknown as Stack;
}

const sampleChange = {
	path: "a.ts",
	pathBytes: [97, 46, 116, 115],
	status: { type: "Modification" },
} as unknown as TreeChange;

function renderSubmenu(opts: {
	stacks: Stack[];
	stackId?: string;
	target?: StageOrAmendTarget;
	label?: string;
	onaction?: () => void;
}) {
	const assignHunk = vi.fn().mockResolvedValue([]);
	const amendCommitMutation = vi.fn().mockResolvedValue({
		newCommit: "new-commit",
		rejectedChanges: [],
	});
	const removeMany = vi.fn();
	const clearPreview = vi.fn();
	const selectionId: SelectionId = { type: "worktree", stackId: opts.stackId };

	const context = new Map<unknown, unknown>([
		[
			STACK_SERVICE._key,
			{
				stacks: () => ({ result: { data: opts.stacks } }),
				amendCommitMutation,
			},
		],
		[DIFF_SERVICE._key, { assignHunk }],
		[
			HOOKS_SERVICE._key,
			{
				runPreCommitHooks: vi.fn(),
				runPostCommitHooks: vi.fn(),
			},
		],
		[
			FILE_SELECTION_MANAGER._key,
			{
				removeMany,
				remove: vi.fn(),
				clearPreview,
			},
		],
		[
			UNCOMMITTED_SERVICE._key,
			{
				at: () => ({
					getAssignmentsByPath: () => [
						{
							hunkHeader: null,
							pathBytes: sampleChange.pathBytes,
						},
					],
					getAssignmentsByPaths: () => ({
						"a.ts": [{ hunkHeader: null, pathBytes: sampleChange.pathBytes }],
					}),
				}),
			},
		],
		[
			UI_STATE._key,
			{
				project: () => ({ stackBusy: { set: vi.fn() } }),
				lane: () => ({ selection: { current: undefined, set: vi.fn() } }),
				global: { modal: { set: vi.fn() } },
			},
		],
	]);

	const view = render(StageOrAmendSubmenu, {
		props: {
			projectId: "proj-1",
			stackId: opts.stackId,
			selectionId,
			target: opts.target ?? { type: "files", changes: [sampleChange] },
			label: opts.label,
			onaction: opts.onaction,
		},
		context,
	});

	return { ...view, assignHunk, amendCommitMutation, removeMany, clearPreview };
}

describe("StageOrAmendSubmenu", () => {
	beforeAll(() => vi.stubGlobal("PointerEvent", MouseEvent));
	afterAll(() => vi.unstubAllGlobals());

	test("lists targetable stacks on level 1 and stages to the chosen stack on level 2", async () => {
		const user = userEvent.setup();
		const onaction = vi.fn();
		const stacks = [
			makeStack("stack-a", [
				{
					branchName: "feature/auth",
					commits: [makeCommit("aaaaaaa1234", "Add login form\n\nBody text")],
				},
			]),
			makeStack("stack-b", [{ branchName: "feature/ui", commits: [] }]),
			makeStack(null, [{ branchName: "unmanaged", commits: [] }]),
		];

		const { assignHunk, amendCommitMutation } = renderSubmenu({ stacks, onaction });

		await user.click(screen.getByText("Stage / Amend"));
		expect(await screen.findByText("feature/auth")).toBeInTheDocument();
		expect(screen.getByText("feature/ui")).toBeInTheDocument();
		expect(screen.queryByText("unmanaged")).not.toBeInTheDocument();

		await user.click(screen.getByText("feature/auth"));
		expect(await screen.findByText("Stage")).toBeInTheDocument();
		expect(screen.getByText("Add login form")).toBeInTheDocument();
		expect(screen.getByText("aaaaaaa")).toBeInTheDocument();

		await user.click(screen.getByText("Stage"));
		expect(onaction).toHaveBeenCalledOnce();
		expect(assignHunk).toHaveBeenCalledExactlyOnceWith({
			projectId: "proj-1",
			assignments: [
				{
					hunkHeader: null,
					pathBytes: sampleChange.pathBytes,
					target: { type: "stack", subject: { stackId: "stack-a" } },
				},
			],
		});
		expect(amendCommitMutation).not.toHaveBeenCalled();
	});

	test("shows Unstage for the stack where changes are already staged and unstages to null target", async () => {
		const user = userEvent.setup();
		const stacks = [
			makeStack("stack-a", [{ branchName: "feature/auth", commits: [] }]),
			makeStack("stack-b", [{ branchName: "feature/ui", commits: [] }]),
		];

		const { assignHunk } = renderSubmenu({ stacks, stackId: "stack-a" });

		await user.click(screen.getByText("Stage / Amend"));
		await user.click(await screen.findByText("feature/auth"));

		expect(await screen.findByText("Unstage")).toBeInTheDocument();
		expect(screen.queryByText("Stage")).not.toBeInTheDocument();

		await user.click(screen.getByText("Unstage"));
		expect(assignHunk).toHaveBeenCalledExactlyOnceWith({
			projectId: "proj-1",
			assignments: [
				{
					hunkHeader: null,
					pathBytes: sampleChange.pathBytes,
					target: null,
				},
			],
		});
	});

	test("amends into a selectable commit and disables conflicted or integrated commits", async () => {
		const user = userEvent.setup();
		const stacks = [
			makeStack("stack-a", [
				{
					branchName: "feature/top",
					commits: [
						makeCommit("1111111aaaa", "Editable commit"),
						makeCommit("2222222bbbb", "Conflicted commit", { hasConflicts: true }),
						makeCommit("3333333cccc", "Integrated commit", { integrated: true }),
					],
				},
				{
					branchName: "feature/base",
					commits: [makeCommit("4444444dddd", "Base commit")],
				},
			]),
		];

		const { amendCommitMutation } = renderSubmenu({ stacks });

		await user.click(screen.getByText("Stage / Amend"));
		await user.click(await screen.findByText("feature/top"));

		expect(await screen.findByText("feature/base")).toBeInTheDocument();

		const conflictedBtn = screen.getByText("Conflicted commit").closest("button")!;
		const integratedBtn = screen.getByText("Integrated commit").closest("button")!;
		expect(conflictedBtn).toBeDisabled();
		expect(integratedBtn).toBeDisabled();

		await user.click(conflictedBtn);
		await user.click(integratedBtn);
		expect(amendCommitMutation).not.toHaveBeenCalled();

		await user.click(screen.getByText("Base commit"));
		expect(amendCommitMutation).toHaveBeenCalledExactlyOnceWith({
			projectId: "proj-1",
			stackId: "stack-a",
			commitId: "4444444dddd",
			worktreeChanges: [
				{
					previousPathBytes: null,
					pathBytes: sampleChange.pathBytes,
					hunkHeaders: [],
				},
			],
			dryRun: false,
		});
	});

	test("keeps an open second-level submenu open when re-hovering its stack trigger", async () => {
		const user = userEvent.setup();
		const stacks = [
			makeStack("stack-a", [
				{
					branchName: "feature/auth",
					commits: [makeCommit("1111111aaaa", "Commit one")],
				},
			]),
		];

		renderSubmenu({ stacks });

		await user.click(screen.getByText("Stage / Amend"));
		const stackButton = (await screen.findByText("feature/auth")).closest("button")!;
		await user.click(stackButton);
		expect(await screen.findByText("Commit one")).toBeInTheDocument();

		await fireEvent.mouseEnter(stackButton);
		expect(screen.getByText("Commit one")).toBeInTheDocument();
	});

	test("renders nothing when there are no targetable stacks", () => {
		renderSubmenu({
			stacks: [makeStack(null, [{ branchName: "detached", commits: [] }])],
		});

		expect(screen.queryByText("Stage / Amend")).not.toBeInTheDocument();
	});

	test("amend-only line selection hides empty stacks and Stage/Unstage options", async () => {
		const user = userEvent.setup();
		const unselectHunkLines = vi.fn();
		const hunk = {
			oldStart: 1,
			oldLines: 1,
			newStart: 1,
			newLines: 3,
			diff: "@@ -1,1 +1,3 @@\n context\n+line one\n+line two\n",
		};
		const stacks = [
			makeStack("stack-a", [
				{
					branchName: "feature/commits",
					commits: [makeCommit("1111111aaaa", "Amendable commit")],
				},
			]),
			makeStack("stack-empty", [{ branchName: "feature/empty", commits: [] }]),
		];

		const { assignHunk, amendCommitMutation } = renderSubmenu({
			stacks,
			stackId: "stack-a",
			label: "Amend with 2 selected lines",
			target: {
				type: "lines",
				change: sampleChange,
				hunk,
				selectedLines: [
					{ oldLine: undefined, newLine: 2 },
					{ oldLine: undefined, newLine: 3 },
				],
				unselectHunkLines,
			},
		});

		await user.click(screen.getByText("Amend with 2 selected lines"));
		expect(await screen.findByText("feature/commits")).toBeInTheDocument();
		expect(screen.queryByText("feature/empty")).not.toBeInTheDocument();

		await user.click(screen.getByText("feature/commits"));
		expect(await screen.findByText("Amendable commit")).toBeInTheDocument();
		expect(screen.queryByText("Stage")).not.toBeInTheDocument();
		expect(screen.queryByText("Unstage")).not.toBeInTheDocument();

		await user.click(screen.getByText("Amendable commit"));
		expect(assignHunk).not.toHaveBeenCalled();
		expect(unselectHunkLines).toHaveBeenCalledExactlyOnceWith(hunk);
		expect(amendCommitMutation).toHaveBeenCalledExactlyOnceWith({
			projectId: "proj-1",
			stackId: "stack-a",
			worktree: undefined,
			commitId: "1111111aaaa",
			worktreeChanges: [
				{
					previousPathBytes: null,
					pathBytes: sampleChange.pathBytes,
					hunkHeaders: [{ oldStart: 0, oldLines: 0, newStart: 2, newLines: 2 }],
				},
			],
			dryRun: false,
		});
	});
});
