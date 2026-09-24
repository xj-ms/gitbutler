<script lang="ts">
	import { BASE_BRANCH_SERVICE } from "$lib/baseBranch/baseBranchService.svelte";
	import { showError } from "$lib/error/showError";
	import { STACK_SERVICE } from "$lib/stacks/stackService.svelte";
	import { inject } from "@gitbutler/core/context";
	import { Button, InfoMessage, Modal, Select, SelectItem } from "@gitbutler/ui";

	const { projectId }: { projectId: string } = $props();

	const TAG_PREFIX = "refs/tags/";

	const stackService = inject(STACK_SERVICE);
	const baseBranchService = inject(BASE_BRANCH_SERVICE);

	const baseBranchQuery = $derived(baseBranchService.baseBranch(projectId));
	const baseBranch = $derived(baseBranchQuery.response);
	const remoteBranchesQuery = $derived(baseBranchService.remoteBranches(projectId));
	const remoteTagsQuery = $derived(baseBranchService.remoteTags(projectId));
	const [setBaseBranchTarget, targetBranchSwitch] = baseBranchService.setTarget;

	// Tags are fully qualified (`refs/tags/<name>`) so they never collide with a
	// remote branch that happens to share the same short name.
	const currentTarget = $derived(
		baseBranch?.isTag ? `${TAG_PREFIX}${baseBranch.branchName}` : baseBranch?.branchName,
	);

	const stacksQuery = $derived(stackService.stacks(projectId));
	const stackCount = $derived(stacksQuery.response?.length);
	const hasActiveStacks = $derived(!!(stackCount && stackCount > 0));

	let modal = $state<Modal>();
	let selectedTarget = $state<string>();

	const options = $derived([
		...(remoteBranchesQuery.response ?? []).map((b) => ({ label: b.name, value: b.name })),
		...(remoteTagsQuery.response ?? []).map((t) => ({
			label: `${t.name} (tag)`,
			value: `${TAG_PREFIX}${t.name}`,
		})),
	]);

	export function show() {
		selectedTarget = currentTarget;
		modal?.show();
	}

	async function onSwitch(close: () => void) {
		if (!selectedTarget || selectedTarget === currentTarget) {
			close();
			return;
		}
		try {
			await setBaseBranchTarget({ projectId, branch: selectedTarget });
			close();
		} catch (err: unknown) {
			showError("Failed to switch base", err);
		}
	}
</script>

<Modal bind:this={modal} title="Switch base target" width="small">
	{#snippet children(_item, close)}
		<div class="switch-base">
			<Select
				value={selectedTarget}
				{options}
				wide
				searchable
				label="Target branch or tag"
				onselect={(value) => {
					selectedTarget = value;
				}}
			>
				{#snippet itemSnippet({ item, highlighted })}
					<SelectItem selected={item.value === selectedTarget} {highlighted}>
						{item.label}
					</SelectItem>
				{/snippet}
			</Select>

			{#if hasActiveStacks}
				<InfoMessage filled outlined={false} icon="info">
					{#snippet content()}
						You have {stackCount === 1 ? "1 active branch" : `${stackCount} active branches`} in your
						workspace. You can switch to a target that is a descendant of the current base; other switches
						require clearing the workspace first.
					{/snippet}
				</InfoMessage>
			{/if}
		</div>
	{/snippet}

	{#snippet controls(close)}
		<Button kind="outline" onclick={close}>Cancel</Button>
		<Button
			style="pop"
			loading={targetBranchSwitch.current.isLoading}
			disabled={!selectedTarget || selectedTarget === currentTarget}
			onclick={() => onSwitch(close)}
		>
			Switch
		</Button>
	{/snippet}
</Modal>

<style lang="postcss">
	.switch-base {
		display: flex;
		flex-direction: column;
		gap: 12px;
	}
</style>
