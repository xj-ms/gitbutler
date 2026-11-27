import { stageTarget } from "$lib/commits/stageOrAmend.svelte";
import {
	FileChangeDropData,
	FolderChangeDropData,
	HunkDropDataV3,
	type ChangeDropData,
} from "$lib/dragging/draggables";
import { type DiffService } from "$lib/hunks/diffService.svelte";
import type { DropzoneHandler } from "$lib/dragging/handler";
import type { FileSelectionManager } from "$lib/selection/fileSelectionManager.svelte";
import type { UncommittedService } from "$lib/selection/uncommittedService.svelte";

export class AssignmentDropHandler implements DropzoneHandler {
	constructor(
		private readonly projectId: string,
		private readonly diffService: DiffService,
		private readonly uncommittedService: UncommittedService,
		private readonly stackId: string | undefined,
		private readonly idSelection: FileSelectionManager,
	) {}

	accepts(data: unknown) {
		if (data instanceof FileChangeDropData || data instanceof FolderChangeDropData) {
			if (data.isCommitted) return false;
			// A linked worktree's changes can't be assigned to stacks.
			if (data.selectionId.worktree !== undefined) return false;
			if (data.stackId === this.stackId) return false;
			return true;
		}
		if (data instanceof HunkDropDataV3) {
			if (!data.uncommitted) return false;
			if (data.selectionId.type !== "worktree") return false;
			if (data.selectionId.stackId === this.stackId) return false;
			return true;
		}
		return false;
	}

	async ondrop(data: ChangeDropData | HunkDropDataV3) {
		if ((data.stackId ?? undefined) === this.stackId) return;
		if (data instanceof FileChangeDropData || data instanceof FolderChangeDropData) {
			const changes = await data.treeChanges();
			await stageTarget({
				projectId: this.projectId,
				sourceStackId: data.stackId,
				selectionId: data.selectionId,
				target: { type: "files", changes },
				targetStackId: this.stackId ?? null,
				deps: {
					diffService: this.diffService,
					idSelection: this.idSelection,
					uncommittedService: this.uncommittedService,
				},
			});
			return;
		}

		await stageTarget({
			projectId: this.projectId,
			sourceStackId: data.stackId,
			selectionId: data.selectionId,
			target: {
				type: "hunk",
				change: data.change,
				hunk: data.hunk,
			},
			targetStackId: this.stackId ?? null,
			deps: {
				diffService: this.diffService,
				idSelection: this.idSelection,
				uncommittedService: this.uncommittedService,
			},
		});
	}
}
