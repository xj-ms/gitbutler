import { splitMessage } from "$lib/commits/commitMessage";
import { changesToDiffSpec, hunkToDiffSpec } from "$lib/commits/utils";
import { projectRunCommitHooks } from "$lib/config/config";
import { HookFailedError, HOOKS_SERVICE, type HooksService } from "$lib/git/hooksService";
import { DIFF_SERVICE, type DiffService } from "$lib/hunks/diffService.svelte";
import {
	diffToHunkHeaders,
	hunkHeaderEquals,
	lineIdsToHunkHeaders,
	orderHeaders,
} from "$lib/hunks/hunk";
import {
	FILE_SELECTION_MANAGER,
	type FileSelectionManager,
} from "$lib/selection/fileSelectionManager.svelte";
import {
	UNCOMMITTED_SERVICE,
	type UncommittedService,
} from "$lib/selection/uncommittedService.svelte";
import { type Stack } from "$lib/stacks/stack";
import { rejectedChangesByPath, type CreateCommitOutcome } from "$lib/stacks/stackEndpoints";
import { STACK_SERVICE, type StackService } from "$lib/stacks/stackService.svelte";
import { UI_STATE, withStackBusy, type UiState } from "$lib/state/uiState.svelte";
import { inject } from "@gitbutler/core/context";
import { get } from "svelte/store";
import type { DropResult } from "$lib/dragging/dropResult";
import type { SelectionId } from "$lib/selection/key";
import type {
	DiffHunk,
	DiffSpec,
	HunkAssignmentTarget,
	HunkHeader,
	TreeChange,
} from "@gitbutler/but-sdk";
import type { LineId } from "@gitbutler/ui-svelte/utils/diffParsing";

const MAX_COMMIT_TITLE_LENGTH = 50;

export type TargetableStack = Stack & { id: string };

export type StageableTarget =
	| {
			type: "files";
			changes: TreeChange[];
	  }
	| {
			type: "hunk";
			change: TreeChange;
			hunk: DiffHunk;
			unselectHunkLines?: (hunk: DiffHunk) => void;
	  }
	| {
			type: "hunk";
			change: TreeChange;
			hunk: HunkHeader;
			unselectHunkLines?: undefined;
	  };

export type AmendLinesTarget = {
	type: "lines";
	change: TreeChange;
	hunk: DiffHunk;
	selectedLines: LineId[];
	unselectHunkLines?: (hunk: DiffHunk) => void;
};

export type StageOrAmendTarget = StageableTarget | AmendLinesTarget;

export type StageOrAmendDependencies = {
	diffService: DiffService;
	hooksService: HooksService;
	idSelection: FileSelectionManager;
	stackService: StackService;
	uiState: UiState;
	uncommittedService: UncommittedService;
};

export function isTargetableStack(stack: Stack): stack is TargetableStack {
	return stack.id !== null;
}

export function stackHasCommits(stack: Stack): boolean {
	return stack.segments.some((segment) => segment.commits.length > 0);
}

export function canAmendCommit(commit: {
	hasConflicts: boolean;
	state: { type: string };
}): boolean {
	return !commit.hasConflicts && commit.state.type !== "Integrated";
}

export function formatCommitTitle(message: string): string {
	const title = splitMessage(message).title || "Empty commit";
	if (title.length > MAX_COMMIT_TITLE_LENGTH) {
		return `${title.slice(0, MAX_COMMIT_TITLE_LENGTH)}…`;
	}
	return title;
}

export async function runAmendWorktreeChanges(params: {
	projectId: string;
	stackId: string | undefined;
	worktree?: string;
	commitId: string;
	targetBranchName?: string;
	worktreeChanges: DiffSpec[];
	runHooks: boolean;
	hooksService: HooksService;
	stackService: StackService;
	uiState: UiState;
	onNewCommit?: (newCommitId: string) => void;
}): Promise<{ outcome?: CreateCommitOutcome; dropResult?: DropResult }> {
	const {
		projectId,
		stackId,
		worktree,
		commitId,
		targetBranchName = "",
		worktreeChanges,
		runHooks,
		hooksService,
		stackService,
		uiState,
		onNewCommit,
	} = params;

	if (worktreeChanges.length === 0) return {};

	let outcome: CreateCommitOutcome | undefined;
	let dropResult: DropResult | undefined;

	await withStackBusy(
		uiState,
		projectId,
		{ commitId, stackIds: stackId ? [stackId] : undefined },
		async () => {
			if (runHooks) {
				try {
					await hooksService.runPreCommitHooks(projectId, worktreeChanges);
				} catch (err) {
					dropResult =
						err instanceof HookFailedError
							? { type: "ok" }
							: { type: "error", title: "Git hook failed", error: err };
					return;
				}
			}

			outcome = await stackService.amendCommitMutation({
				projectId,
				stackId,
				worktree,
				commitId,
				worktreeChanges,
				dryRun: false,
			});

			if (outcome.newCommit) {
				if (onNewCommit) {
					onNewCommit(outcome.newCommit);
				} else if (stackId) {
					const laneSelection = uiState.lane(stackId).selection;
					if (laneSelection.current?.commitId === commitId) {
						laneSelection.set({
							...laneSelection.current,
							commitId: outcome.newCommit,
						});
					}
				}
			}

			if (outcome.rejectedChanges.length > 0) {
				dropResult = {
					type: "rejectedChanges",
					projectId,
					newCommitId: outcome.newCommit ?? undefined,
					commitTitle: undefined,
					targetBranchName,
					pathsToRejectedChanges: rejectedChangesByPath(outcome),
				};
			}

			if (runHooks) {
				await hooksService.runPostCommitHooks(projectId);
			}
		},
	);

	return { outcome, dropResult };
}

export async function amendWorktreeChanges(params: {
	projectId: string;
	stackId: string;
	commitId: string;
	targetBranchName?: string;
	worktreeChanges: DiffSpec[];
	runHooks: boolean;
	hooksService: HooksService;
	stackService: StackService;
	uiState: UiState;
}): Promise<CreateCommitOutcome | undefined> {
	const { outcome, dropResult } = await runAmendWorktreeChanges(params);

	if (dropResult?.type === "error") {
		const { showError } = await import("$lib/error/showError");
		showError(dropResult.title, dropResult.error);
	} else if (dropResult?.type === "rejectedChanges") {
		params.uiState.global.modal.set({
			type: "commit-failed",
			projectId: dropResult.projectId,
			targetBranchName: dropResult.targetBranchName,
			newCommitId: dropResult.newCommitId,
			commitTitle: dropResult.commitTitle,
			pathsToRejectedChanges: dropResult.pathsToRejectedChanges,
		});
	}

	return outcome;
}

export async function stageTarget(params: {
	projectId: string;
	sourceStackId: string | null | undefined;
	selectionId: SelectionId;
	target: StageableTarget;
	targetStackId: string | null;
	deps: Pick<StageOrAmendDependencies, "diffService" | "idSelection" | "uncommittedService">;
}): Promise<void> {
	const { projectId, sourceStackId, selectionId, target, targetStackId, deps } = params;
	const assignmentTarget: HunkAssignmentTarget | null = targetStackId
		? { type: "stack", subject: { stackId: targetStackId } }
		: null;

	if (target.type === "files") {
		const assignments = target.changes
			.flatMap((change) =>
				deps.uncommittedService.getAssignmentsByPath(sourceStackId || null, change.path),
			)
			.map((assignment) => ({
				hunkHeader: assignment.hunkHeader,
				pathBytes: assignment.pathBytes,
				target: assignmentTarget,
			}));
		if (assignments.length === 0) return;

		await deps.diffService.assignHunk({ projectId, assignments });

		const movedFiles = target.changes.map((change) => ({ ...selectionId, path: change.path }));
		deps.idSelection.removeMany(movedFiles);
		deps.idSelection.clearPreview(selectionId);
		return;
	}

	const { change, hunk } = target;
	const allAssignments = deps.uncommittedService.getAssignmentsByPath(
		sourceStackId ?? null,
		change.path,
	);
	const assignment = allAssignments.find((candidate) =>
		candidate.hunkHeader === null ? true : hunkHeaderEquals(hunk, candidate.hunkHeader),
	);
	if (!assignment) return;

	if (target.unselectHunkLines) {
		target.unselectHunkLines(target.hunk);
	}
	await deps.diffService.assignHunk({
		projectId,
		assignments: [
			{
				hunkHeader: assignment.hunkHeader,
				pathBytes: assignment.pathBytes,
				target: assignmentTarget,
			},
		],
	});

	if (allAssignments.length === 1) {
		deps.idSelection.remove(change.path, selectionId);
		deps.idSelection.clearPreview(selectionId);
	}
}

export async function amendTarget(params: {
	projectId: string;
	sourceStackId: string | null | undefined;
	selectionId: SelectionId;
	target: StageOrAmendTarget;
	targetStackId: string;
	commitId: string;
	targetBranchName?: string;
	runHooks: boolean;
	deps: Omit<StageOrAmendDependencies, "diffService">;
}): Promise<void> {
	const {
		projectId,
		sourceStackId,
		selectionId,
		target,
		targetStackId,
		commitId,
		targetBranchName,
		runHooks,
		deps,
	} = params;

	if (target.type === "files") {
		const assignments = deps.uncommittedService.getAssignmentsByPaths(
			sourceStackId ?? null,
			target.changes.map((change) => change.path),
		);
		const worktreeChanges = changesToDiffSpec(target.changes, assignments);
		const outcome = await amendWorktreeChanges({
			projectId,
			stackId: targetStackId,
			commitId,
			targetBranchName,
			worktreeChanges,
			runHooks,
			hooksService: deps.hooksService,
			stackService: deps.stackService,
			uiState: deps.uiState,
		});
		if (!outcome) return;

		const rejectedPaths = new Set(outcome.rejectedChanges.map((rejected) => rejected.path));
		const amendedFiles = target.changes
			.filter((change) => !rejectedPaths.has(change.path))
			.map((change) => ({ ...selectionId, path: change.path }));
		if (amendedFiles.length > 0) {
			deps.idSelection.removeMany(amendedFiles);
			deps.idSelection.clearPreview(selectionId);
		}
		return;
	}

	if (target.type === "lines") {
		if (target.selectedLines.length === 0) return;

		const { change, hunk, selectedLines } = target;
		const hunkHeaders = lineIdsToHunkHeaders(selectedLines, hunk.diff, "commit");
		if (hunkHeaders.length === 0) return;
		hunkHeaders.sort(orderHeaders);

		const allAssignments = deps.uncommittedService.getAssignmentsByPath(
			sourceStackId ?? null,
			change.path,
		);
		const allHunkHeaders = diffToHunkHeaders(hunk.diff, "commit");
		allHunkHeaders.sort(orderHeaders);
		const coversEntireHunk =
			hunkHeaders.length === allHunkHeaders.length &&
			hunkHeaders.every((header, index) => {
				const candidate = allHunkHeaders[index];
				return candidate !== undefined && hunkHeaderEquals(header, candidate);
			});

		const previousPathBytes =
			change.status.type === "Rename" ? change.status.subject.previousPathBytes : null;

		target.unselectHunkLines?.(hunk);

		const outcome = await amendWorktreeChanges({
			projectId,
			stackId: targetStackId,
			commitId,
			targetBranchName,
			worktreeChanges: [
				{
					previousPathBytes,
					pathBytes: change.pathBytes,
					hunkHeaders,
				},
			],
			runHooks,
			hooksService: deps.hooksService,
			stackService: deps.stackService,
			uiState: deps.uiState,
		});

		if (
			outcome &&
			outcome.rejectedChanges.length === 0 &&
			allAssignments.length === 1 &&
			coversEntireHunk
		) {
			deps.idSelection.remove(change.path, selectionId);
			deps.idSelection.clearPreview(selectionId);
		}
		return;
	}

	const { change, hunk } = target;
	const allAssignments = deps.uncommittedService.getAssignmentsByPath(
		sourceStackId ?? null,
		change.path,
	);

	if (target.unselectHunkLines) {
		target.unselectHunkLines(target.hunk);
	}
	const outcome = await amendWorktreeChanges({
		projectId,
		stackId: targetStackId,
		commitId,
		targetBranchName,
		worktreeChanges: [hunkToDiffSpec(change, hunk)],
		runHooks,
		hooksService: deps.hooksService,
		stackService: deps.stackService,
		uiState: deps.uiState,
	});

	if (outcome && outcome.rejectedChanges.length === 0 && allAssignments.length === 1) {
		deps.idSelection.remove(change.path, selectionId);
		deps.idSelection.clearPreview(selectionId);
	}
}

export function createStageOrAmendController(
	getContextParams: () => {
		projectId: string;
		stackId: string | undefined;
		selectionId: SelectionId;
	},
) {
	const stackService = inject(STACK_SERVICE);
	const diffService = inject(DIFF_SERVICE);
	const hooksService = inject(HOOKS_SERVICE);
	const idSelection = inject(FILE_SELECTION_MANAGER);
	const uncommittedServices = inject(UNCOMMITTED_SERVICE);
	const uiState = inject(UI_STATE);

	const stacksQuery = $derived(stackService.stacks(getContextParams().projectId));
	const targetableStacks = $derived((stacksQuery.result.data ?? []).filter(isTargetableStack));

	return {
		get targetableStacks() {
			return targetableStacks;
		},
		stage(target: StageableTarget, targetStackId: string | null) {
			const { projectId, stackId, selectionId } = getContextParams();
			const uncommittedService = uncommittedServices.at(selectionId.worktree);
			return stageTarget({
				projectId,
				sourceStackId: stackId,
				selectionId,
				target,
				targetStackId,
				deps: { diffService, idSelection, uncommittedService },
			});
		},
		amend(
			target: StageOrAmendTarget,
			targetStackId: string,
			commitId: string,
			targetBranchName?: string,
		) {
			const { projectId, stackId, selectionId } = getContextParams();
			const uncommittedService = uncommittedServices.at(selectionId.worktree);
			const runHooks = get(projectRunCommitHooks(projectId));
			return amendTarget({
				projectId,
				sourceStackId: stackId,
				selectionId,
				target,
				targetStackId,
				commitId,
				targetBranchName,
				runHooks,
				deps: {
					hooksService,
					idSelection,
					stackService,
					uiState,
					uncommittedService,
				},
			});
		},
	};
}
