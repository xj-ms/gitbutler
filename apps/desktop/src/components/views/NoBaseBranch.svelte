<script lang="ts">
	import ProjectSetup from "$components/onboarding/ProjectSetup.svelte";
	import FullviewLoading from "$components/shared/FullviewLoading.svelte";
	import ProblemLoadingRepo from "$components/views/ProblemLoadingRepo.svelte";
	import { BASE_BRANCH_SERVICE } from "$lib/baseBranch/baseBranchService.svelte";
	import { inject } from "@gitbutler/core/context";

	const { projectId }: { projectId: string } = $props();
	const baseBranchService = inject(BASE_BRANCH_SERVICE);
	const remoteBranchesQuery = $derived(baseBranchService.remoteBranches(projectId));
	const remoteTagsQuery = $derived(baseBranchService.remoteTags(projectId));
</script>

{#if remoteBranchesQuery.result.isLoading || remoteTagsQuery.result.isLoading}
	<!--TODO: Add project id -->
	<FullviewLoading />
{:else if remoteBranchesQuery.result.isSuccess}
	{@const remoteBranches = remoteBranchesQuery.response ?? []}
	{@const remoteTags = remoteTagsQuery.response ?? []}
	{#if remoteBranches.length === 0 && remoteTags.length === 0}
		<ProblemLoadingRepo
			{projectId}
			error="Currently, GitButler requires a remote branch or tag to base its virtual branch work on. To
						use virtual branches, please push your code to a remote branch or create a tag to use as a base"
		/>
	{:else}
		<ProjectSetup {projectId} {remoteBranches} {remoteTags} />
	{/if}
{:else if remoteBranchesQuery.result.isError}
	<ProblemLoadingRepo
		{projectId}
		error="Currently, GitButler requires a remote branch or tag to base its virtual branch work on. To
						use virtual branches, please push your code to a remote branch or create a tag to use as a base"
	/>
{/if}
