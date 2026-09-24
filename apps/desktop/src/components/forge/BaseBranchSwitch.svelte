<script lang="ts">
	import { BASE_BRANCH_SERVICE } from "$lib/baseBranch/baseBranchService.svelte";
	import { STACK_SERVICE } from "$lib/stacks/stackService.svelte";
	import { inject } from "@gitbutler/core/context";
	import { Button, CardGroup, InfoMessage, Select, SelectItem } from "@gitbutler/ui-svelte";

	const { projectId }: { projectId: string } = $props();

	const TAG_PREFIX = "refs/tags/";

	const stackService = inject(STACK_SERVICE);
	const baseBranchService = inject(BASE_BRANCH_SERVICE);
	const baseBranchQuery = $derived(baseBranchService.baseBranch(projectId));
	const baseBranch = $derived(baseBranchQuery.response);
	const remoteBranchesQuery = $derived(baseBranchService.remoteBranches(projectId));
	const localBranchesQuery = $derived(baseBranchService.localTargetBranches(projectId));
	const localBranches = $derived(localBranchesQuery.response ?? []);
	const remoteTagsQuery = $derived(baseBranchService.remoteTags(projectId));
	const [setBaseBranchTargetRef, targetRefSwitch] = baseBranchService.setTargetRef;

	const currentTarget = $derived(
		baseBranch?.isTag
			? `${TAG_PREFIX}${baseBranch.branchName}`
			: localBranches.find((branch) => branch.isTarget)?.refName,
	);

	let selectedTarget = $derived(currentTarget);
	let selectedRemote = $derived(baseBranch?.pushRemoteName ?? undefined);

	const isTagSelected = $derived(!!selectedTarget?.startsWith(TAG_PREFIX));

	const stacksQuery = $derived(stackService.stacks(projectId));
	const stackCount = $derived(stacksQuery.response?.length);
	const hasActiveStacks = $derived(!!(stackCount && stackCount > 0));

	function uniqueRemotes(remoteBranches: { name: string }[]): { name: string }[] {
		return Array.from(new Set(remoteBranches.map((b) => b.name.split("/")[0])))
			.filter((name): name is string => !!name)
			.map((r) => ({
				name: r,
			}));
	}

	const switching = $derived(targetRefSwitch.current.isLoading);

	function targetOptions(
		branches: { name: string; refName: string }[],
		tags: { name: string }[],
	) {
		return [
			...branches.map((b) => ({ label: b.name, value: b.refName })),
			...tags.map((t) => ({ label: `${t.name} (tag)`, value: `${TAG_PREFIX}${t.name}` })),
		];
	}

	async function switchTarget(targetRef: string, pushRemote?: string) {
		// Only update the target; the user keeps working on their current branch.
		await setBaseBranchTargetRef({ projectId, targetRef, pushRemote });
	}

	async function onSetBaseBranchClick() {
		if (!selectedTarget) return;
		const pushRemote = isTagSelected ? undefined : selectedRemote;
		await switchTarget(selectedTarget, pushRemote);
	}
</script>

{#if remoteBranchesQuery.result.isLoading || localBranchesQuery.result.isLoading || remoteTagsQuery.result.isLoading}
	<InfoMessage filled outlined={false} icon="info">
		{#snippet content()}
			Loading branches...
		{/snippet}
	</InfoMessage>
{:else if remoteBranchesQuery.result.isSuccess && localBranchesQuery.result.isSuccess}
	{@const remoteBranches = remoteBranchesQuery.response ?? []}
	{@const remoteTags = remoteTagsQuery.response ?? []}
	{#if localBranches.length > 0 || remoteTags.length > 0}
		{@const remotes = uniqueRemotes(remoteBranches)}
		{@const options = targetOptions(localBranches, remoteTags)}
		<CardGroup>
			<CardGroup.Item>
				{#snippet title()}
					Remote configuration
				{/snippet}
				{#snippet caption()}
					Lets you choose where to push code and set the target for contributions. The target is a
					local branch like 'master' or 'main' (whose configured upstream is used for pulling), or a
					tag, which pins the base to a fixed commit and disables pushing to the base.
				{/snippet}

				<Select
					value={selectedTarget}
					{options}
					wide
					onselect={(value) => {
						selectedTarget = value;
					}}
					label="Current target"
					searchable
				>
					{#snippet itemSnippet({ item, highlighted })}
						<SelectItem selected={item.value === selectedTarget} {highlighted}>
							{item.label}
						</SelectItem>
					{/snippet}
				</Select>

				{#if remotes.length > 1 && !isTagSelected}
					<Select
						value={selectedRemote}
						options={remotes.map((r) => ({ label: r.name, value: r.name }))}
						wide
						onselect={(value) => {
							selectedRemote = value;
						}}
						label="Create branches on remote"
					>
						{#snippet itemSnippet({ item, highlighted })}
							<SelectItem selected={item.value === selectedRemote} {highlighted}>
								{item.label}
							</SelectItem>
						{/snippet}
					</Select>
				{/if}

				{#if hasActiveStacks}
					<InfoMessage filled outlined={false} icon="info">
						{#snippet content()}
							You have {stackCount === 1 ? "1 active branch" : `${stackCount} active branches`} in your
							workspace. You can switch to a target that is a descendant of the current base; other switches
							require clearing the workspace first.
						{/snippet}
					</InfoMessage>
				{/if}

				<Button
					kind="outline"
					onclick={onSetBaseBranchClick}
					id="set-base-branch"
					loading={switching}
					disabled={!selectedTarget ||
						(selectedTarget === currentTarget &&
							(isTagSelected || selectedRemote === (baseBranch?.pushRemoteName ?? undefined)))}
				>
					{switching ? "Updating target..." : "Update configuration"}
				</Button>
			</CardGroup.Item>
		</CardGroup>
	{/if}
{:else if remoteBranchesQuery.result.isError || localBranchesQuery.result.isError}
	<InfoMessage filled outlined={true} style="danger">
		{#snippet title()}
			We got an error trying to list your branches
		{/snippet}
	</InfoMessage>
{/if}
