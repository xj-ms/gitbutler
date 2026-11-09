import { hunkHeaderEquals } from "$lib/hunks/hunk";
import { InjectionToken } from "@gitbutler/core/context";
import { isDefined } from "@gitbutler/ui/utils/typeguards";
import type { UnifiedDiff } from "$lib/hunks/diff";
import type { BackendApi } from "$lib/state/backendApi";
import type { AppDispatch } from "$lib/state/clientState.svelte";
import type { HunkAssignment, HunkAssignmentRequest, TreeChange } from "@gitbutler/but-sdk";

function bytesEqual(a: number[], b: number[]): boolean {
	if (a.length !== b.length) return false;
	for (let i = 0; i < a.length; i++) {
		if (a[i] !== b[i]) return false;
	}
	return true;
}

function applyHunkAssignmentRequests(
	hunkAssignments: HunkAssignment[],
	requests: HunkAssignmentRequest[],
) {
	for (const req of requests) {
		const targetStackId = req.target?.type === "stack" ? req.target.subject.stackId : null;
		const targetBranchRefBytes =
			req.target?.type === "branch" ? req.target.subject.branchRefBytes : null;
		for (const assignment of hunkAssignments) {
			if (!bytesEqual(assignment.pathBytes, req.pathBytes)) continue;
			if (req.hunkHeader !== null) {
				if (
					assignment.hunkHeader === null ||
					!hunkHeaderEquals(assignment.hunkHeader, req.hunkHeader)
				) {
					continue;
				}
			}
			assignment.stackId = targetStackId;
			assignment.branchRefBytes = targetBranchRefBytes;
		}
	}
}

export type ChangeDiff = {
	path: string;
	diff: UnifiedDiff | null;
};

export const DIFF_SERVICE = new InjectionToken<DiffService>("DiffService");

export class DiffService {
	constructor(
		private backendApi: BackendApi,
		private dispatch: AppDispatch,
	) {}

	getDiff(projectId: string, change: TreeChange) {
		return this.backendApi.endpoints.getDiff.useQuery({ projectId, change });
	}

	assignHunk = async (args: { projectId: string; assignments: HunkAssignmentRequest[] }) => {
		const patchResult = this.dispatch(
			this.backendApi.util.updateQueryData(
				"worktreeChanges",
				{ projectId: args.projectId },
				(draft) => {
					applyHunkAssignmentRequests(draft.hunkAssignments, args.assignments);
				},
			),
		);
		try {
			return await this.backendApi.endpoints.assignHunk.mutate(args);
		} catch (err) {
			patchResult.undo();
			throw err;
		}
	};

	async fetchDiff(projectId: string, change: TreeChange) {
		const { getDiff } = this.backendApi.endpoints;
		return await getDiff.fetch({ projectId, change });
	}

	getChanges(projectId: string, changes: TreeChange[]) {
		const args = changes.map((change) => ({ projectId, change }));
		const { getDiff } = this.backendApi.endpoints;
		return getDiff.useQueries(args, {
			transform: (data, args): ChangeDiff => ({ path: args.change.path, diff: data }),
		});
	}

	async fetchChanges(projectId: string, changes: TreeChange[]): Promise<ChangeDiff[]> {
		const args = changes.map((change) => ({ projectId, change }));
		const responses = await Promise.all(
			args.map((arg) =>
				this.backendApi.endpoints.getDiff.fetch(arg, {
					transform: (diff, args) => ({
						path: args.change.path,
						diff,
					}),
				}),
			),
		);
		return responses.filter(isDefined);
	}
}
