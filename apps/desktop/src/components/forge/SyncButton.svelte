<script lang="ts">
	import BaseBranchSwitchModal from "$components/forge/BaseBranchSwitchModal.svelte";
	import { BASE_BRANCH_SERVICE } from "$lib/baseBranch/baseBranchService.svelte";
	import { BRANCH_SERVICE } from "$lib/branches/branchService.svelte";
	import { FORGE_INFO_SERVICE } from "$lib/forge/forgeInfo.svelte";
	import { LISTING_SERVICE } from "$lib/forge/listingService.svelte";
	import { inject } from "@gitbutler/core/context";
	import { Button, TimeAgo, TestId } from "@gitbutler/ui";

	interface Props {
		projectId: string;
		disabled?: boolean;
	}

	const { projectId, disabled = false }: Props = $props();

	const baseBranchService = inject(BASE_BRANCH_SERVICE);
	const branchService = inject(BRANCH_SERVICE);
	const baseBranch = $derived(baseBranchService.baseBranch(projectId));
	const fetchStatus = $derived(baseBranchService.fetchStatus(projectId));

	const listingService = inject(LISTING_SERVICE);
	const forgeInfoService = inject(FORGE_INFO_SERVICE);

	// `list_reviews` errors for forges without listing support (Bitbucket/
	// Azure) or when no forge can be derived, so only refresh the listing
	// when the forge actually supports it.
	const forgeInfoQuery = $derived(forgeInfoService.get(projectId));
	const canListReviews = $derived(forgeInfoQuery.response?.capabilities.listService ?? false);

	const lastFetched = $derived(
		fetchStatus.response?.lastSuccessfulMs
			? new Date(fetchStatus.response.lastSuccessfulMs)
			: undefined,
	);

	const isTagBase = $derived(baseBranch.response?.isTag ?? false);
	const targetLabel = $derived.by(() => {
		const base = baseBranch.response;
		if (!base) return undefined;
		return base.isTag ? base.shortName : `${base.remoteName}/${base.shortName}`;
	});

	let loading = $state(false);
	let switchModal = $state<BaseBranchSwitchModal>();
</script>

<div class="sync-button">
	{#if baseBranch.response}
		<Button
			testId={TestId.SwitchBaseBranchButton}
			kind="outline"
			width="auto"
			tooltip="Switch base branch or tag"
			icon={isTagBase ? "tag" : "target-branch"}
			reversedDirection
			{disabled}
			onclick={(e: MouseEvent) => {
				e.preventDefault();
				e.stopPropagation();
				switchModal?.show();
			}}
		>
			<span>{targetLabel}</span>
		</Button>
	{/if}

	<Button
		testId={TestId.SyncButton}
		kind="outline"
		width="auto"
		tooltip="Last fetch from upstream"
		{loading}
		{disabled}
		icon="refresh"
		reversedDirection
		onclick={async (e: MouseEvent) => {
			e.preventDefault();
			e.stopPropagation();
			loading = true;
			try {
				await baseBranchService.fetchFromRemotes(projectId, "modal");
				await Promise.all([
					...(canListReviews ? [listingService.refresh(projectId)] : []),
					fetchStatus.result?.refetch(),
					baseBranch.result?.refetch(),
					branchService.refresh(),
				]);
			} finally {
				loading = false;
			}
		}}
	>
		<span>
			{#if loading}
				Fetching...
			{:else if lastFetched}
				<TimeAgo date={lastFetched} addSuffix={true} capitalize={true} />
			{:else}
				Refetch
			{/if}
		</span>
	</Button>
</div>

<BaseBranchSwitchModal bind:this={switchModal} {projectId} />

<style lang="postcss">
	.sync-button {
		display: inline-flex;
		align-items: center;
		gap: 4px;
	}
</style>
