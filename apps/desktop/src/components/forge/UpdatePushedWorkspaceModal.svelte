<script lang="ts">
	import { STACK_SERVICE } from "$lib/stacks/stackService.svelte";
	import { inject } from "@gitbutler/core/context";
	import {
		Badge,
		Button,
		Icon,
		Modal,
		ScrollableContainer,
		TestId,
		chipToasts,
	} from "@gitbutler/ui-svelte";
	import type { PushedWorkspaceUpdatePlan } from "$lib/stacks/stackEndpoints";

	type Props = {
		projectId: string;
	};

	const { projectId }: Props = $props();

	const stackService = inject(STACK_SERVICE);
	const [updateWorkspaceMutation, updateWorkspaceQuery] = stackService.updatePushedWorkspace;

	let modal = $state<ReturnType<typeof Modal>>();
	let loadingPlan = $state(false);
	let planError = $state<string | null>(null);
	let fetchedPlan = $state<PushedWorkspaceUpdatePlan | null | undefined>(undefined);

	const planQuery = $derived(stackService.pushedWorkspaceUpdatePlan(projectId));
	const plan = $derived(fetchedPlan !== undefined ? fetchedPlan : (planQuery?.response ?? null));
	const isUpdating = $derived(updateWorkspaceQuery.current.isLoading);

	function shortSha(sha: string | null | undefined): string {
		if (!sha) return "(none)";
		return sha.slice(0, 12);
	}

	function formatRefTransition(oldSha: string | null, newSha: string): string {
		if (!oldSha) {
			return `(new) -> ${shortSha(newSha)}`;
		}
		if (oldSha === newSha) {
			return `${shortSha(newSha)} (unchanged)`;
		}
		return `${shortSha(oldSha)} -> ${shortSha(newSha)}`;
	}

	async function executeUpdate() {
		if (!plan || plan.hasUncommittedChanges) return;
		const result = await updateWorkspaceMutation({ projectId });
		chipToasts.success(
			`Updated workspace and ${result.branches.length} ${result.branches.length === 1 ? "branch" : "branches"}`,
		);
		modal?.close();
	}

	export async function show() {
		fetchedPlan = undefined;
		planError = null;
		loadingPlan = true;
		modal?.show();
		try {
			fetchedPlan = (await stackService.fetchPushedWorkspaceUpdatePlan(projectId)) ?? null;
		} catch (err: any) {
			planError = err?.message ?? String(err);
		} finally {
			loadingPlan = false;
		}
	}

	export function close() {
		modal?.close();
	}
</script>

<Modal
	bind:this={modal}
	title="Update pushed workspace"
	width={520}
	testId={TestId.UpdatePushedWorkspaceModal}
	onSubmit={async () => {
		await executeUpdate();
	}}
>
	<div class="content-wrap">
		{#if loadingPlan && !plan}
			<div class="empty-state text-12 clr-text-2">Inspecting upstream workspace commit…</div>
		{:else if planError}
			<div class="error-banner text-12 clr-text-danger">
				{planError}
			</div>
		{:else if !plan}
			<div class="empty-state text-12 clr-text-2">
				Workspace is already up to date with its remote tracking branch.
			</div>
		{:else}
			{#if plan.hasUncommittedChanges}
				<div class="error-banner">
					<div class="error-banner__header">
						<Icon name="warning" color="var(--fill-danger-bg)" />
						<span class="text-12 text-semibold clr-text-danger">
							Working tree has uncommitted changes in tracked files. Please commit, stash, or
							discard them before updating the workspace.
						</span>
					</div>
					<div class="dirty-files text-11 clr-text-2">
						{#each plan.uncommittedFiles.slice(0, 8) as file (file)}
							<div class="truncate">• {file}</div>
						{/each}
						{#if plan.uncommittedFiles.length > 8}
							<div>…and {plan.uncommittedFiles.length - 8} more</div>
						{/if}
					</div>
				</div>
			{/if}

			<div class="summary-section">
				<div class="branches-header">
					<span class="text-12 text-semibold clr-text-2">Workspace &amp; base target</span>
				</div>
				<div class="scroll-wrap">
					<div class="branch-list">
						<div class="branch-row">
							<div class="branch-row__left">
								<Icon name="branch" color="var(--text-2)" />
								<span class="text-12 clr-text-2">Upstream ref</span>
							</div>
							<div class="branch-row__right">
								<span class="text-12 text-semibold">{plan.upstreamRefName}</span>
							</div>
						</div>

						<div class="branch-row">
							<div class="branch-row__left">
								<Icon name="commit" color="var(--text-2)" />
								<span class="text-12 clr-text-2">Workspace HEAD</span>
							</div>
							<div class="branch-row__right">
								<Badge style="pop">
									{formatRefTransition(plan.oldWorkspaceSha, plan.newWorkspaceSha)}
								</Badge>
							</div>
						</div>

						<div class="branch-row">
							<div class="branch-row__left">
								<Icon
									name={plan.target.newRefName.startsWith("refs/tags/") ? "tag" : "target-branch"}
									color="var(--text-2)"
								/>
								<span class="text-12 clr-text-2 truncate" title={plan.target.newRefName}>
									Base ref ({plan.target.newRefName})
								</span>
							</div>
							<div class="branch-row__right">
								<Badge
									style={plan.target.oldRefSha === plan.target.newTargetCommitId
										? "gray"
										: "warning"}
								>
									{formatRefTransition(plan.target.oldRefSha, plan.target.newTargetCommitId)}
								</Badge>
							</div>
						</div>
					</div>
				</div>
			</div>

			{#if plan.submodules.length > 0}
				<div class="branches-section">
					<div class="branches-header">
						<span class="text-12 text-semibold clr-text-2">Submodules to update</span>
						<Badge>{plan.submodules.length}</Badge>
					</div>
					<div class="scroll-wrap">
						<div class="branch-list">
							{#each plan.submodules as sub (sub.name)}
								<div class="branch-row">
									<div class="branch-row__left">
										<Icon name="repo" color="var(--text-2)" />
										<span class="text-13 text-semibold truncate">{sub.name}</span>
									</div>
									<div class="branch-row__right">
										<Badge style="warning">
											{formatRefTransition(sub.oldSha, sub.newSha)}
										</Badge>
									</div>
								</div>
							{/each}
						</div>
					</div>
				</div>
			{/if}

			<div class="branches-section">
				<div class="branches-header">
					<span class="text-12 text-semibold clr-text-2">Applied branches to reset</span>
					<Badge>{plan.branches.length}</Badge>
				</div>

				<div class="scroll-wrap">
					<ScrollableContainer maxHeight="14rem">
						<div class="branch-list">
							{#each plan.branches as branch, idx (branch.name)}
								<div class="branch-row">
									<div class="branch-row__left">
										<span class="text-11 clr-text-3">[{idx + 1}]</span>
										<Icon name="branch-local" color="var(--text-2)" />
										<span class="text-13 text-semibold truncate">{branch.name}</span>
									</div>
									<div class="branch-row__right">
										<Badge
											style={!branch.oldSha
												? "pop"
												: branch.oldSha === branch.newSha
													? "gray"
													: "warning"}
										>
											{formatRefTransition(branch.oldSha, branch.newSha)}
										</Badge>
									</div>
								</div>
							{/each}
						</div>
					</ScrollableContainer>
				</div>
			</div>
		{/if}
	</div>

	{#snippet controls(close)}
		<Button kind="outline" type="reset" onclick={close}>Cancel</Button>
		<Button
			testId={TestId.UpdatePushedWorkspaceActionButton}
			style="pop"
			type="submit"
			disabled={!plan || plan.hasUncommittedChanges || loadingPlan}
			loading={isUpdating}
		>
			Reset workspace &amp; branches
		</Button>
	{/snippet}
</Modal>

<style lang="postcss">
	.content-wrap {
		display: flex;
		flex-direction: column;
		gap: 16px;
	}

	.summary-section,
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

	.error-banner {
		display: flex;
		flex-direction: column;
		padding: 10px 12px;
		gap: 6px;
		border: 1px solid var(--fill-danger-bg);
		border-radius: var(--radius-m);
		background-color: var(--bg-danger);
	}

	.error-banner__header {
		display: flex;
		align-items: flex-start;
		gap: 8px;
	}

	.dirty-files {
		display: flex;
		flex-direction: column;
		padding-left: 22px;
		gap: 2px;
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
</style>
