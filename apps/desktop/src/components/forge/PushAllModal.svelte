<script lang="ts">
	import BranchNameTextbox from "$components/branch/BranchNameTextbox.svelte";
	import { projectRunCommitHooks } from "$lib/config/config";
	import {
		branchHasConflicts,
		branchHasUnpushedCommits,
		branchRequiresForcePush,
		getColorFromPushStatus,
		pushStatusToIcon,
	} from "$lib/stacks/stack";
	import { STACK_SERVICE } from "$lib/stacks/stackService.svelte";
	import { UI_STATE } from "$lib/state/uiState.svelte";
	import { inject } from "@gitbutler/core/context";
	import {
		Badge,
		Button,
		Icon,
		Modal,
		ScrollableContainer,
		TestId,
		chipToasts,
	} from "@gitbutler/ui";
	import type { PushStatus, Segment } from "@gitbutler/but-sdk";

	type Props = {
		projectId: string;
	};

	const { projectId }: Props = $props();

	const stackService = inject(STACK_SERVICE);
	const uiState = inject(UI_STATE);

	const runHooks = $derived(projectRunCommitHooks(projectId));
	const [pushAllMutation, pushAllQuery] = stackService.pushAll;

	const allStacksQuery = $derived(stackService.stacks(projectId));
	const allStacks = $derived(allStacksQuery?.response ?? []);

	const trackingBranchQuery = $derived(stackService.workspaceTrackingBranch(projectId));
	const remoteName = $derived(trackingBranchQuery?.response?.remoteName ?? "origin");

	type OutOfSyncBranchItem = {
		name: string;
		segment: Segment;
		pushStatus: PushStatus;
		requiresForce: boolean;
		hasConflicts: boolean;
	};

	const outOfSyncBranches = $derived.by(() => {
		const seen = new Set<string>();
		const items: OutOfSyncBranchItem[] = [];
		for (const stack of allStacks) {
			for (const segment of stack.segments) {
				const name = segment.refName?.displayName;
				if (!name || seen.has(name)) continue;
				if (!branchHasUnpushedCommits(segment)) continue;
				seen.add(name);
				items.push({
					name,
					segment,
					pushStatus: segment.pushStatus,
					requiresForce: branchRequiresForcePush(segment),
					hasConflicts: branchHasConflicts(segment),
				});
			}
		}
		return items;
	});

	const hasConflictedBranches = $derived(outOfSyncBranches.some((b) => b.hasConflicts));

	let modal = $state<ReturnType<typeof Modal>>();
	let forcePushProtectionModal = $state<ReturnType<typeof Modal>>();
	let workspaceRemoteBranch = $state<string>("");
	let normalizedWorkspaceBranch = $state<string | undefined>();
	let isBranchNameValid = $state(false);

	function statusLabel(status: PushStatus): string {
		switch (status) {
			case "completelyUnpushed":
				return "New branch";
			case "unpushedCommitsRequiringForce":
				return "Force push";
			case "unpushedCommits":
				return "Unpushed commits";
			default:
				return "Out of sync";
		}
	}

	async function executePush(skipForcePushProtection: boolean) {
		if (!normalizedWorkspaceBranch) return;
		try {
			const pushResult = await pushAllMutation({
				projectId,
				workspaceRemoteBranch: normalizedWorkspaceBranch,
				skipForcePushProtection,
				runHooks: $runHooks,
				pushOpts: [],
			});

			const upstreamBranchNames = pushResult.branchToRemote
				.filter(([localName]) => localName !== "gitbutler/workspace")
				.map(([, , name]) => name);
			if (upstreamBranchNames.length > 0) {
				uiState.project(projectId).branchesToPoll.add(...upstreamBranchNames);
			}

			const count = pushResult.branchToRemote.length;
			chipToasts.success(
				count === 1
					? `Pushed workspace to ${remoteName}/${normalizedWorkspaceBranch}`
					: `Pushed workspace and ${count - 1} ${count - 1 === 1 ? "branch" : "branches"} successfully`,
			);
			modal?.close();
		} catch (error: any) {
			if (error?.code === "GitForcePushProtection") {
				modal?.close();
				forcePushProtectionModal?.show();
				return;
			}
			throw error;
		}
	}

	const isPushing = $derived(pushAllQuery.current.isLoading);

	export async function show() {
		modal?.show();
		try {
			const info = await stackService.fetchWorkspaceTrackingBranch(projectId);
			workspaceRemoteBranch = info?.branchName ?? "";
		} catch {
			workspaceRemoteBranch = trackingBranchQuery?.response?.branchName ?? "";
		}
	}

	export function close() {
		modal?.close();
	}
</script>

<Modal
	bind:this={modal}
	title="Push all branches"
	width={500}
	testId={TestId.PushAllModal}
	onSubmit={async () => {
		await executePush(false);
	}}
>
	<div class="content-wrap">
		<BranchNameTextbox
			label="Workspace remote tracking branch ({remoteName})"
			placeholder="e.g. gitbutler/workspace"
			bind:value={workspaceRemoteBranch}
			helperText={`Pushes gitbutler/workspace to ${remoteName}/${normalizedWorkspaceBranch ?? (workspaceRemoteBranch || "<branch>")}`}
			autofocus
			onnormalizedvalue={(val) => (normalizedWorkspaceBranch = val)}
			onvalidationchange={(valid) => (isBranchNameValid = valid)}
		/>

		<div class="branches-section">
			<div class="branches-header">
				<span class="text-12 text-semibold clr-text-2">Out-of-sync applied branches</span>
				<Badge>{outOfSyncBranches.length}</Badge>
			</div>

			{#if outOfSyncBranches.length === 0}
				<div class="empty-state text-12 clr-text-2">
					All applied branches are up to date with their remote. Only
					<span class="text-semibold">gitbutler/workspace</span> will be pushed.
				</div>
			{:else}
				<div class="scroll-wrap">
					<ScrollableContainer maxHeight="14rem">
						<div class="branch-list">
							{#each outOfSyncBranches as branch (branch.name)}
								<div class="branch-row">
									<div class="branch-row__left">
										<Icon
											name={pushStatusToIcon(branch.pushStatus)}
											color={getColorFromPushStatus(branch.pushStatus)}
										/>
										<span class="text-13 text-semibold truncate">{branch.name}</span>
									</div>
									<div class="branch-row__right">
										{#if branch.hasConflicts}
											<Badge style="danger">Conflicted</Badge>
										{/if}
										<Badge style={branch.requiresForce ? "warning" : "gray"}>
											{statusLabel(branch.pushStatus)}
										</Badge>
									</div>
								</div>
							{/each}
						</div>
					</ScrollableContainer>
				</div>
			{/if}

			{#if hasConflictedBranches}
				<p class="text-12 clr-text-danger">Please resolve any conflicted commits before pushing.</p>
			{/if}
		</div>
	</div>

	{#snippet controls(close)}
		<Button kind="outline" type="reset" onclick={close}>Cancel</Button>
		<Button
			testId={TestId.PushAllModalActionButton}
			style="pop"
			type="submit"
			disabled={!isBranchNameValid || hasConflictedBranches}
			loading={isPushing}
		>
			Push all
		</Button>
	{/snippet}
</Modal>

<Modal
	title="Protected force push"
	width={480}
	type="warning"
	bind:this={forcePushProtectionModal}
	onSubmit={async (close) => {
		close();
		await executePush(true);
	}}
>
	<p class="text-13">
		Your force push was blocked because a remote branch contains commits your local branch doesn’t
		include. To prevent overwriting history, <span class="text-bold"
			>cancel and pull & integrate</span
		>
		the changes.
	</p>

	{#snippet controls(close)}
		<div class="controls">
			<Button kind="outline" type="submit" loading={isPushing}>Force push anyway</Button>
			<Button wide style="pop" onclick={close}>Cancel</Button>
		</div>
	{/snippet}
</Modal>

<style lang="postcss">
	.content-wrap {
		display: flex;
		flex-direction: column;
		gap: 16px;
	}

	.branches-section {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.branches-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
	}

	.empty-state {
		padding: 12px;
		border: 1px solid var(--border-2);
		border-radius: var(--radius-m);
		background-color: var(--bg-2);
	}

	.scroll-wrap {
		overflow: hidden;
		border: 1px solid var(--border-2);
		border-radius: var(--radius-m);
	}

	.branch-list {
		display: flex;
		flex-direction: column;
	}

	.branch-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 12px;
		gap: 12px;
		border-bottom: 1px solid var(--border-2);

		&:last-child {
			border-bottom: none;
		}
	}

	.branch-row__left {
		display: flex;
		align-items: center;
		overflow: hidden;
		gap: 8px;
	}

	.branch-row__right {
		display: flex;
		flex-shrink: 0;
		align-items: center;
		gap: 6px;
	}

	.controls {
		display: flex;
		width: 100%;
		gap: 6px;
	}
</style>
