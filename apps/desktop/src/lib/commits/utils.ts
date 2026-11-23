import { isDefined } from "@gitbutler/ui-svelte/utils/typeguards";
import type { DiffSpec, HunkAssignment, HunkHeader, TreeChange } from "@gitbutler/but-sdk";

/** Helper function that turns tree changes into a diff spec */
export function changesToDiffSpec(
	changes: TreeChange[],
	assignments?: Record<string, HunkAssignment[]>,
): DiffSpec[] {
	return changes.map((change) => {
		const previousPathBytes =
			change.status.type === "Rename" ? change.status.subject.previousPathBytes : null;
		const assignment = assignments?.[change.path];
		const hunkHeaders = assignment?.map((a) => a.hunkHeader).filter(isDefined) ?? [];

		return {
			previousPathBytes,
			pathBytes: change.pathBytes,
			hunkHeaders,
		};
	});
}

/** Turns a single hunk of a tree change into a diff spec. */
export function hunkToDiffSpec(change: TreeChange, hunk: HunkHeader): DiffSpec {
	const previousPathBytes =
		change.status.type === "Rename" ? change.status.subject.previousPathBytes : null;
	const isWholeFileChange = change.status.type === "Addition" || change.status.type === "Deletion";

	return {
		previousPathBytes,
		pathBytes: change.pathBytes,
		hunkHeaders: isWholeFileChange
			? []
			: [
					{
						oldStart: hunk.oldStart,
						oldLines: hunk.oldLines,
						newStart: hunk.newStart,
						newLines: hunk.newLines,
					},
				],
	};
}

export function findEarliestConflict<T extends { hasConflicts?: boolean }>(
	commits: T[],
): T | undefined {
	if (!commits.length) return undefined;

	for (let i = commits.length - 1; i >= 0; i--) {
		const commit = commits[i]!;
		if (commit.hasConflicts) {
			return commit;
		}
	}

	return undefined;
}
