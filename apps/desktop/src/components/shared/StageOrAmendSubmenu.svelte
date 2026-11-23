<script lang="ts">
	import {
		canAmendCommit,
		createStageOrAmendController,
		formatCommitTitle,
		stackHasCommits,
		type StageOrAmendTarget,
	} from "$lib/commits/stageOrAmend.svelte";
	import { getStackName } from "$lib/stacks/stack";
	import {
		ContextMenuItem,
		ContextMenuItemSubmenu,
		ContextMenuSection,
		type IconName,
	} from "@gitbutler/ui-svelte";
	import type { SelectionId } from "$lib/selection/key";

	type Props = {
		projectId: string;
		stackId?: string;
		selectionId: SelectionId;
		target: StageOrAmendTarget;
		label?: string;
		icon?: IconName;
		disabled?: boolean;
		testId?: string;
		onaction?: () => void;
	};

	const {
		projectId,
		stackId,
		selectionId,
		target,
		label = "Stage / Amend",
		icon = "lanes",
		disabled = false,
		testId,
		onaction,
	}: Props = $props();

	const controller = createStageOrAmendController(() => ({
		projectId,
		stackId,
		selectionId,
	}));

	const visibleStacks = $derived(
		target.type === "lines"
			? controller.targetableStacks.filter(stackHasCommits)
			: controller.targetableStacks,
	);
</script>

{#if visibleStacks.length > 0}
	<ContextMenuItemSubmenu {label} {icon} {disabled} {testId}>
		{#snippet submenu({ close: closeStacksMenu })}
			<ContextMenuSection>
				{#each visibleStacks as stack (stack.id)}
					{@const stackName = getStackName(stack)}
					{@const isStagedInStack = stackId === stack.id}
					{@const segmentsWithCommits = stack.segments.filter(
						(segment) => segment.commits.length > 0,
					)}
					{@const showSegmentTitles = stack.segments.length > 1}
					<ContextMenuItemSubmenu label={stackName}>
						{#snippet submenu({ close: closeActionsMenu })}
							{#if target.type !== "lines"}
								{@const stageableTarget = target}
								<ContextMenuSection>
									<ContextMenuItem
										label={isStagedInStack ? "Unstage" : "Stage"}
										onclick={() => {
											closeActionsMenu();
											closeStacksMenu();
											const promise = controller.stage(
												stageableTarget,
												isStagedInStack ? null : stack.id,
											);
											onaction?.();
											return promise;
										}}
									/>
								</ContextMenuSection>
							{/if}
							{#each segmentsWithCommits as segment, index (segment.refName?.fullNameBytes?.join(",") ?? index)}
								<ContextMenuSection
									title={showSegmentTitles ? segment.refName?.displayName : undefined}
								>
									{#each segment.commits as commit (commit.id)}
										<ContextMenuItem
											label={formatCommitTitle(commit.message)}
											disabled={!canAmendCommit(commit)}
											onclick={() => {
												closeActionsMenu();
												closeStacksMenu();
												const promise = controller.amend(
													target,
													stack.id,
													commit.id,
													segment.refName?.displayName,
												);
												onaction?.();
												return promise;
											}}
										>
											{#snippet control()}
												<span class="commit-sha text-11">{commit.id.slice(0, 7)}</span>
											{/snippet}
										</ContextMenuItem>
									{/each}
								</ContextMenuSection>
							{/each}
						{/snippet}
					</ContextMenuItemSubmenu>
				{/each}
			</ContextMenuSection>
		{/snippet}
	</ContextMenuItemSubmenu>
{/if}

<style lang="postcss">
	.commit-sha {
		margin-left: 8px;
		color: var(--text-3);
		font-family: var(--font-mono);
	}
</style>
