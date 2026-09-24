<script lang="ts">
	import { BASE_BRANCH_SERVICE } from "$lib/baseBranch/baseBranchService.svelte";
	import { SETTINGS_SERVICE } from "$lib/settings/appSettings";
	import { STACK_SERVICE } from "$lib/stacks/stackService.svelte";
	import { inject } from "@gitbutler/core/context";
	import { Button, CardGroup, InfoMessage, Select, SelectItem } from "@gitbutler/ui";

	const { projectId }: { projectId: string } = $props();

	const TAG_PREFIX = "refs/tags/";

	const stackService = inject(STACK_SERVICE);
	const baseBranchService = inject(BASE_BRANCH_SERVICE);
	const settingsStore = inject(SETTINGS_SERVICE).appSettings;
	const baseBranchQuery = $derived(baseBranchService.baseBranch(projectId));
	const baseBranch = $derived(baseBranchQuery.response);
	const remoteBranchesQuery = $derived(baseBranchService.remoteBranches(projectId));
	const remoteTagsQuery = $derived(baseBranchService.remoteTags(projectId));
	const [setBaseBranchTarget, targetBranchSwitch] = baseBranchService.setTarget;
	const [setBaseBranchTargetRef, targetRefSwitch] = baseBranchService.setTargetRef;

	// The current target as a selector value: tags are fully qualified (`refs/tags/<name>`)
	// so they never collide with a remote branch of the same short name.
	const currentTarget = $derived(
		baseBranch?.isTag ? `${TAG_PREFIX}${baseBranch.branchName}` : baseBranch?.branchName,
	);

	let selectedTarget = $derived(currentTarget);
	let selectedRemote = $derived(baseBranch?.pushRemoteName);

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

	const switching = $derived(
		targetBranchSwitch.current.isLoading || targetRefSwitch.current.isLoading,
	);
	// With the singleBranch feature flag, only the target metadata is rewritten
	// and no branch is checked out, so avoid claiming a branch switch.
	const switchingLabel = $derived(
		$settingsStore?.featureFlags.singleBranch ? "Updating target..." : "Switching...",
	);

	function targetOptions(branches: { name: string }[], tags: { name: string }[]) {
		return [
			...branches.map((b) => ({ label: b.name, value: b.name })),
			...tags.map((t) => ({ label: `${t.name} (tag)`, value: `${TAG_PREFIX}${t.name}` })),
		];
	}

	async function switchTarget(target: string, pushRemote?: string) {
		if ($settingsStore?.featureFlags.singleBranch) {
			const targetRef = target.startsWith(TAG_PREFIX) ? target : `refs/remotes/${target}`;
			await setBaseBranchTargetRef({ projectId, targetRef, pushRemote });
		} else {
			await setBaseBranchTarget({ projectId, branch: target, pushRemote });
		}
	}

	async function onSetBaseBranchClick() {
		if (!selectedTarget) return;
		const pushRemote = isTagSelected ? undefined : selectedRemote;
		await switchTarget(selectedTarget, pushRemote);
	}
</script>

{#if remoteBranchesQuery.result.isLoading}
	<InfoMessage filled outlined={false} icon="info">
		{#snippet content()}
			Loading remote branches...
		{/snippet}
	</InfoMessage>
{:else if remoteBranchesQuery.result.isSuccess}
	{@const remoteBranches = remoteBranchesQuery.response}
	{@const remoteTags = remoteTagsQuery.response ?? []}
	{#if remoteBranches && remoteBranches.length > 0}
		{@const remotes = uniqueRemotes(remoteBranches)}
		{@const options = targetOptions(remoteBranches, remoteTags)}
		<CardGroup>
			<CardGroup.Item>
				{#snippet title()}
					Remote configuration
				{/snippet}
				{#snippet caption()}
					Lets you choose where to push code and set the target for contributions. The target is
					usually the "production" branch like 'origin/master' or 'upstream/main', but may also be a
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
					disabled={selectedTarget === currentTarget &&
						(isTagSelected || selectedRemote === baseBranch?.pushRemoteName)}
				>
					{switching ? switchingLabel : "Update configuration"}
				</Button>
			</CardGroup.Item>
		</CardGroup>
	{/if}
{:else if remoteBranchesQuery.result.isError}
	<InfoMessage filled outlined={true} style="danger">
		{#snippet title()}
			We got an error trying to list your remote branches
		{/snippet}
	</InfoMessage>
{/if}
